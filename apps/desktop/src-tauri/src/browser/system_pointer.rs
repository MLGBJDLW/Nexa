//! Physical input for the foreground, windowed WebView2 only. The controller
//! parent is wry's dedicated tab container, distinct from the host window.
use nexa_core::browser_runtime::{
    acquire_desktop_input_permit, try_acquire_cross_process_input, CrossProcessDesktopInputGuard,
    ForegroundInputWatch,
};
use tauri::Webview;
use windows::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::ClientToScreen,
    UI::{
        HiDpi::{
            SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        },
        Input::KeyboardAndMouse::*,
        WindowsAndMessaging::*,
    },
};

struct Pixels(DPI_AWARENESS_CONTEXT);
impl Pixels {
    fn enter() -> Result<Self, String> {
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous.0.is_null() {
            return Err("Cannot establish physical WebView coordinates.".into());
        }
        Ok(Self(previous))
    }
}
impl Drop for Pixels {
    fn drop(&mut self) {
        unsafe { SetThreadDpiAwarenessContext(self.0) };
    }
}

pub(super) struct SystemPointer {
    container: isize,
    main: isize,
    rect: RECT,
    scale: f64,
    viewport: serde_json::Value,
    pub watch: ForegroundInputWatch,
    _local: tokio::sync::OwnedMutexGuard<()>,
    _shared: CrossProcessDesktopInputGuard,
}

fn screen_rect(container: HWND) -> Result<RECT, String> {
    let mut rect = RECT::default();
    let mut origin = POINT::default();
    unsafe { GetClientRect(container, &mut rect) }.map_err(|e| e.to_string())?;
    if !unsafe { ClientToScreen(container, &mut origin) }.as_bool() {
        return Err("Cannot locate WebView client origin.".into());
    }
    Ok(RECT {
        left: origin.x,
        top: origin.y,
        right: origin.x + rect.right,
        bottom: origin.y + rect.bottom,
    })
}

fn mouse(flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}
fn send(events: &[INPUT]) -> Result<(), String> {
    let sent = unsafe { SendInput(events, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == events.len() {
        Ok(())
    } else {
        Err(format!(
            "Windows delivered {sent} of {} browser pointer events.",
            events.len()
        ))
    }
}
fn cursor() -> Result<POINT, String> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p) }.map_err(|e| e.to_string())?;
    Ok(p)
}

impl SystemPointer {
    pub fn observe_foreground(webview: &Webview) -> Option<ForegroundInputWatch> {
        let main = webview.window().hwnd().ok()?;
        if unsafe { GetForegroundWindow() }.0 != main.0 {
            return None;
        }
        ForegroundInputWatch::start().ok()
    }

    pub async fn for_action(
        webview: &Webview,
        watch: Option<ForegroundInputWatch>,
        observation: &serde_json::Value,
        prepared: &serde_json::Value,
        modifiers: &[String],
    ) -> Result<Option<Self>, String> {
        let live = &prepared["systemViewport"];
        if !modifiers.is_empty()
            || live["visualScale"].as_f64() != Some(1.)
            || live["visualOffsetX"].as_f64() != Some(0.)
            || live["visualOffsetY"].as_f64() != Some(0.)
        {
            // Modifier chords can trigger system shortcuts. Pixel wheel and
            // pinch/pan use the WebView's own viewport coordinate system.
            return Ok(None);
        }
        if [
            "width",
            "height",
            "deviceScaleFactor",
            "visualScale",
            "visualOffsetX",
            "visualOffsetY",
        ]
        .iter()
        .any(|field| observation.get(*field) != live.get(*field))
        {
            return Err("Browser viewport changed since observation. Observe again.".into());
        }
        Self::acquire(webview, live, watch).await
    }

    async fn acquire(
        webview: &Webview,
        viewport: &serde_json::Value,
        observation_watch: Option<ForegroundInputWatch>,
    ) -> Result<Option<Self>, String> {
        let main = webview.window().hwnd().map_err(|e| e.to_string())?.0 as isize;
        if unsafe { GetForegroundWindow() }.0 as isize != main {
            return Ok(None);
        }
        let watch = observation_watch
            .ok_or("Observe the foreground WebView again before moving the system cursor.")?;
        if watch.changed() {
            return Err("User input or input monitoring changed since the browser observation. Observe again.".into());
        }
        let local = acquire_desktop_input_permit()
            .await
            .map_err(|e| e.to_string())?;
        let shared = try_acquire_cross_process_input().map_err(|e| e.to_string())?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        webview.with_webview(move |platform| {
            let result = (|| -> Result<(isize, RECT), String> {
                let _pixels = Pixels::enter()?;
                let controller = platform.controller();
                let mut parent = Default::default();
                let mut bounds = Default::default();
                let mut visible = Default::default();
                unsafe {
                    controller.ParentWindow(&mut parent)
                        .and_then(|_| controller.Bounds(&mut bounds))
                        .and_then(|_| controller.IsVisible(&mut visible))
                }.map_err(|error| error.to_string())?;
                let container = HWND(parent.0);
                let rect = screen_rect(container)?;
                if !visible.as_bool() || container.0 as isize == main
                    || unsafe { GetAncestor(container, GA_ROOT) }.0 as isize != main
                    || bounds.left != 0 || bounds.top != 0
                    || bounds.right != rect.right - rect.left
                    || bounds.bottom != rect.bottom - rect.top {
                    return Err("WebView2 does not expose a visible, dedicated tab client with matching controller bounds.".into());
                }
                Ok((container.0 as isize, rect))
            })();
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())?;
        let (container, rect) = tokio::time::timeout(std::time::Duration::from_secs(3), rx)
            .await
            .map_err(|_| "WebView2 geometry timed out.")?
            .map_err(|_| "WebView2 geometry was interrupted.")??;
        let dimension = |key: &str| {
            viewport[key]
                .as_f64()
                .filter(|n| n.is_finite() && *n > 0.)
                .ok_or("Browser observation lacks valid viewport geometry.")
        };
        let scale = dimension("deviceScaleFactor")?;
        let tolerance = scale.max(1.) + 1.;
        if (dimension("width")? * scale - f64::from(rect.right - rect.left)).abs() > tolerance
            || (dimension("height")? * scale - f64::from(rect.bottom - rect.top)).abs() > tolerance
        {
            return Err("WebView2 physical bounds and observed CSS viewport no longer agree. Observe again.".into());
        }
        Ok(Some(Self {
            container,
            main,
            rect,
            scale,
            viewport: viewport.clone(),
            watch,
            _local: local,
            _shared: shared,
        }))
    }
    fn checked_point(&self, css: (f64, f64)) -> Result<POINT, String> {
        let container = HWND(self.container as *mut _);
        let main = HWND(self.main as *mut _);
        if self.watch.changed()
            || unsafe { GetForegroundWindow() } != main
            || !unsafe { IsWindowVisible(container) }.as_bool()
            || !unsafe { IsWindowEnabled(main) }.as_bool()
            || screen_rect(container)? != self.rect
        {
            return Err(
                "The user, active foreground window, or WebView bounds changed. Observe again."
                    .into(),
            );
        }
        let dx = (css.0 * self.scale).round();
        let dy = (css.1 * self.scale).round();
        if !dx.is_finite()
            || !dy.is_finite()
            || dx < 0.0
            || dy < 0.0
            || dx >= f64::from(self.rect.right - self.rect.left)
            || dy >= f64::from(self.rect.bottom - self.rect.top)
        {
            return Err("The observed point is outside the WebView client.".into());
        }
        let point = POINT {
            x: self.rect.left + dx as i32,
            y: self.rect.top + dy as i32,
        };
        let hit = unsafe { WindowFromPoint(point) };
        if !css.0.is_finite()
            || !css.1.is_finite()
            || point.x < self.rect.left
            || point.y < self.rect.top
            || point.x >= self.rect.right
            || point.y >= self.rect.bottom
            || !unsafe { IsChild(container, hit) }.as_bool()
        {
            return Err("The physical pointer target is outside the selected WebView or covered by another surface.".into());
        }
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        unsafe { GetGUIThreadInfo(0, &mut info) }.map_err(|e| e.to_string())?;
        if !info.hwndMenuOwner.0.is_null()
            || (!info.hwndCapture.0.is_null()
                && !unsafe { IsChild(container, info.hwndCapture) }.as_bool())
        {
            return Err("Another window or menu owns the system pointer.".into());
        }
        Ok(point)
    }
    pub fn move_to(&self, css: (f64, f64)) -> Result<(), String> {
        let _pixels = Pixels::enter()?;
        let p = self.checked_point(css)?;
        let movement = nexa_core::browser_runtime::absolute_pointer_input((p.x, p.y))
            .map_err(|e| e.to_string())?;
        send(&[movement])?;
        self.verify_cursor(css)
    }
    pub fn verify_cursor(&self, css: (f64, f64)) -> Result<(), String> {
        let _pixels = Pixels::enter()?;
        let p = self.checked_point(css)?;
        let actual = cursor()?;
        if actual.x != p.x || actual.y != p.y {
            return Err(format!("The system cursor left the observed WebView target ({}, {}); current coordinates are ({}, {}).", p.x,p.y,actual.x,actual.y));
        }
        Ok(())
    }
    pub fn check_idle(&self) -> Result<(), String> {
        if [
            VK_LBUTTON,
            VK_MBUTTON,
            VK_RBUTTON,
            VK_XBUTTON1,
            VK_XBUTTON2,
            VK_CONTROL,
            VK_SHIFT,
            VK_MENU,
            VK_LWIN,
            VK_RWIN,
        ]
        .into_iter()
        .any(|v| unsafe { GetAsyncKeyState(i32::from(v.0)) } < 0)
        {
            return Err(
                "Release the held mouse buttons and modifiers before the browser pointer gesture."
                    .into(),
            );
        }
        Ok(())
    }
    pub async fn gesture(
        &mut self,
        guard: &super::webview_host::ArmedTrustedInputGuard,
        original_url: &url::Url,
        action: &str,
        button: &str,
        start: (f64, f64),
        end: Option<(f64, f64)>,
        revalidate: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        let count = if action == "double_click" { 2 } else { 1 };
        let destination = end.unwrap_or(start);
        if action == "drag" && (destination.0 - start.0).hypot(destination.1 - start.1) < 8. {
            return Err("Drag targets must be at least 8 CSS pixels apart.".into());
        }
        for click in 1..=count {
            guard.check_system_target(&self.viewport, false).await?;
            revalidate()?;
            let held = self.press(start, button)?;
            if action == "drag" {
                for step in 1..=16 {
                    tokio::time::sleep(std::time::Duration::from_millis(8)).await;
                    revalidate()?;
                    let t = f64::from(step) / 16.;
                    self.move_to((
                        start.0 + (destination.0 - start.0) * t,
                        start.1 + (destination.1 - start.1) * t,
                    ))?;
                }
            }
            if action == "drag" {
                guard.check_system_target(&self.viewport, true).await?;
            }
            revalidate()?;
            self.verify_cursor(destination)?;
            held.release()?;
            guard
                .await_system_release(click, original_url, click < count)
                .await?;
        }
        Ok(())
    }
    fn press(&self, css: (f64, f64), button: &str) -> Result<HeldButton, String> {
        self.check_idle()?;
        self.verify_cursor(css)?;
        let (down, up) = match button {
            "left" => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            "middle" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
            "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            _ => return Err("Unsupported system mouse button.".into()),
        };
        send(&[mouse(down, 0, 0)])?;
        Ok(HeldButton(Some(up)))
    }
}

pub(super) struct HeldButton(Option<MOUSE_EVENT_FLAGS>);
impl HeldButton {
    pub fn release(mut self) -> Result<(), String> {
        let flag = self.0.expect("pressed button");
        send(&[mouse(flag, 0, 0)])?;
        self.0 = None;
        Ok(())
    }
}
impl Drop for HeldButton {
    fn drop(&mut self) {
        if let Some(up) = self.0 {
            let _ = send(&[mouse(up, 0, 0)]);
        }
    }
}
