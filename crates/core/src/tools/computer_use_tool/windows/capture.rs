//! One bounded capture owner, with a same-window Win32 fallback for WGC failures.
use super::*;
use std::sync::{mpsc, Arc, Mutex};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetWindowDC,
    PatBlt, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLACKNESS,
    DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowDisplayAffinity, GetWindowRect, PW_RENDERFULLCONTENT,
};
use windows_capture::{
    capture::{Context, GraphicsCaptureApiHandler},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
};

type FrameResult = Result<RgbaImage, String>;
type FrameSender = mpsc::SyncSender<FrameResult>;

pub(super) struct CaptureFrame {
    pub image: RgbaImage,
    pub backend: &'static str,
}

fn pool(
) -> Result<&'static super::super::super::computer_capture_lifecycle::CaptureCleanupPool, CoreError>
{
    use super::super::super::computer_capture_lifecycle::CaptureCleanupPool;
    static POOL: OnceLock<Result<CaptureCleanupPool, String>> = OnceLock::new();
    POOL.get_or_init(|| CaptureCleanupPool::new(2))
        .as_ref()
        .map_err(|error| invalid(error.clone()))
}

fn run_capture_job(
    operation: impl FnOnce(FrameSender) -> Result<(), String> + Send + 'static,
    timeout: Duration,
) -> FrameResult {
    let pool = pool().map_err(|error| error.to_string())?;
    let permit = pool.acquire()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let _completion = pool.dispatch(permit, move || operation(sender))?;
    // This includes the upstream startup handshake, which itself has no deadline.
    receiver
        .recv_timeout(timeout)
        .map_err(|error| format!("Native capture startup/frame unavailable: {error}"))?
}

struct DiagnosticItem {
    window: Window,
    error: Arc<Mutex<Option<String>>>,
}
impl TryInto<GraphicsCaptureItemType> for DiagnosticItem {
    type Error = windows::core::Error;
    fn try_into(self) -> Result<GraphicsCaptureItemType, Self::Error> {
        self.window
            .try_into()
            .inspect_err(|error: &windows::core::Error| {
                if let Ok(mut detail) = self.error.lock() {
                    *detail = Some(format!("{error} (HRESULT 0x{:08X})", error.code().0 as u32));
                }
            })
    }
}

struct OneFrameCapture {
    sender: FrameSender,
}
impl GraphicsCaptureApiHandler for OneFrameCapture {
    type Flags = FrameSender;
    type Error = String;
    fn new(context: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            sender: context.flags,
        })
    }
    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let (width, height) = (frame.width(), frame.height());
        let result = (|| {
            let pixels = u64::from(width) * u64::from(height);
            if pixels == 0 || pixels > MAX_NATIVE_CAPTURE_PIXELS {
                return Err(format!(
                    "Capture frame {width}x{height} exceeds the native pixel budget"
                ));
            }
            let buffer = frame
                .buffer()
                .map_err(|error| format!("Read capture frame: {error}"))?;
            let mut compact = Vec::new();
            RgbaImage::from_raw(
                width,
                height,
                buffer.as_nopadding_buffer(&mut compact).to_vec(),
            )
            .ok_or_else(|| "Capture frame had an invalid RGBA buffer".into())
        })();
        let _ = self.sender.try_send(result);
        control.stop();
        Ok(())
    }
}

fn wgc(window: Window, output: FrameSender) -> Result<(), String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let diagnostic = Arc::new(Mutex::new(None));
    let settings = Settings::new(
        DiagnosticItem {
            window,
            error: diagnostic.clone(),
        },
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        sender,
    );
    let control = match OneFrameCapture::start_free_threaded(settings) {
        Ok(control) => control,
        Err(error) => {
            let detail = diagnostic.lock().ok().and_then(|detail| detail.clone());
            let _ = output.try_send(Err(format!(
                "Windows Graphics Capture startup: {error}{}",
                detail
                    .map(|detail| format!(": {detail}"))
                    .unwrap_or_default()
            )));
            // Upstream joined its failed startup thread; this slot is reusable.
            return Ok(());
        }
    };
    match receiver.recv_timeout(Duration::from_secs(2)) {
        Ok(frame) => {
            let _ = output.try_send(frame);
            control
                .wait()
                .map_err(|error| format!("Join capture worker: {error}"))
        }
        Err(error) => {
            let _ = output.try_send(Err(format!("Windows Graphics Capture frame: {error}")));
            control
                .stop()
                .map_err(|error| format!("Stop capture worker: {error}"))
        }
    }
}

pub(super) fn capture_rgba(
    window: Window,
    expected: &WindowSnapshot,
) -> Result<CaptureFrame, CoreError> {
    capture_with_primary(expected, move |sender| wgc(window, sender))
}

fn capture_with_primary(
    expected: &WindowSnapshot,
    primary: impl FnOnce(FrameSender) -> Result<(), String> + Send + 'static,
) -> Result<CaptureFrame, CoreError> {
    ensure_visible(expected)?;
    match run_capture_job(primary, Duration::from_secs(3)) {
        Ok(image) => Ok(CaptureFrame {
            image,
            backend: "windows_graphics_capture",
        }),
        Err(primary_error) => {
            // Validate again after a delayed driver failure. Never switch HWNDs,
            // restore a hidden window, or capture desktop pixels as a substitute.
            let (_, current) = current_window_for_access(expected, true)?;
            ensure_visible(&current)?;
            if !same_capture_surface(expected, &current) {
                return Err(invalid(
                    "Window changed during capture startup. Refresh its observation.",
                ));
            }
            let expected = current.clone();
            let fallback = run_capture_job(
                move |sender| {
                    let result = print_window(&expected).map_err(|error| error.to_string());
                    let _ = sender.try_send(result);
                    Ok(())
                },
                Duration::from_secs(2),
            );
            fallback
                .map(|image| CaptureFrame {
                    image,
                    backend: "print_window",
                })
                .map_err(|error| {
                    platform_error(
                        "capture exact window",
                        format!("{primary_error}; same-window fallback: {error}"),
                    )
                })
        }
    }
}

pub(super) fn ensure_visible(window: &WindowSnapshot) -> Result<(), CoreError> {
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    if !unsafe { IsWindow(Some(hwnd(window.id))).as_bool() } {
        return Err(CoreError::NotFound(format!(
            "Window {} no longer exists; refresh the window inventory.",
            window.id
        )));
    }
    if window.minimized || !unsafe { IsWindowVisible(hwnd(window.id)).as_bool() } {
        return Err(invalid(format!(
            "Window {} is minimized or hidden; restore the intended window before observing it.",
            window.id
        )));
    }
    let mut cloaked = 0_u32;
    unsafe {
        DwmGetWindowAttribute(
            hwnd(window.id),
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    }
    .map_err(|error| platform_error("check compositor window visibility", error))?;
    if cloaked != 0 {
        return Err(invalid(format!("Window {} is cloaked by Windows; select its visible window or desktop before observing it.", window.id)));
    }
    Ok(())
}

struct GdiFrame {
    window: HWND,
    source: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    original: HGDIOBJ,
}
impl Drop for GdiFrame {
    fn drop(&mut self) {
        unsafe {
            if !self.original.0.is_null() && self.original.0 as isize != -1 {
                SelectObject(self.memory, self.original);
            }
            if !self.bitmap.0.is_null() {
                let _ = DeleteObject(self.bitmap.into());
            }
            if !self.memory.0.is_null() {
                let _ = DeleteDC(self.memory);
            }
            if !self.source.0.is_null() {
                ReleaseDC(Some(self.window), self.source);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    #[test]
    #[ignore = "requires an interactive Windows desktop; observes only an isolated helper"]
    fn windows_capture_falls_back_on_wgc_startup_failure_without_changing_target() {
        let title = format!("Nexa Capture Fallback {}", uuid::Uuid::new_v4().simple());
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "tools::computer_use_tool::tests::windows_computer_control_helper_window",
                "--nocapture",
            ])
            .env("NEXA_COMPUTER_USE_HELPER", "1")
            .env("NEXA_COMPUTER_USE_HELPER_TITLE", &title)
            .env("NEXA_COMPUTER_USE_HELPER_BACKGROUND", "1")
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let result = (|| -> Result<(), String> {
            let deadline = Instant::now() + Duration::from_secs(10);
            let target = loop {
                if let Some(target) = list_windows()
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .find(|window| window.title == title)
                {
                    break target;
                }
                if Instant::now() >= deadline {
                    return Err("Capture fixture did not appear".into());
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            let baseline = capture_rgba(Window::from_raw_hwnd(hwnd(target.id).0), &target)
                .map_err(|error| error.to_string())?;
            let started = Instant::now();
            let fallback = capture_with_primary(&target, |sender| {
                let _ = sender.send(Err(
                    "Failed to convert item to GraphicsCaptureItem (injected startup failure)"
                        .into(),
                ));
                Ok(())
            })
            .map_err(|error| error.to_string())?;
            if fallback.backend != "print_window"
                || fallback.image.dimensions() != (target.width, target.height)
            {
                return Err("Fallback changed the visible frame or did not run".into());
            }
            let primary_png = resized_png(baseline.image.clone())
                .map_err(|error| error.to_string())?
                .0;
            let fallback_png = resized_png(fallback.image.clone())
                .map_err(|error| error.to_string())?
                .0;
            let consistent = screenshot_signature(&primary_png)
                .zip(screenshot_signature(&fallback_png))
                .is_some_and(|(primary, fallback)| {
                    screenshot_signatures_match(&primary, &fallback)
                });
            let fallback_elapsed_ms = started.elapsed().as_millis();
            let (release, blocked) = mpsc::channel();
            let startup_started = Instant::now();
            let delayed = capture_with_primary(&target, move |sender| {
                let _ = blocked.recv();
                let _ = sender.try_send(Err(
                    "Delayed WGC startup finished after caller timeout".into()
                ));
                Ok(())
            });
            let _ = release.send(());
            let delayed = delayed.map_err(|error| error.to_string())?;
            let startup_wait_ms = startup_started.elapsed().as_millis();
            if startup_wait_ms > 5500 || delayed.backend != "print_window" {
                return Err("WGC startup was not bounded with a same-target fallback".into());
            }
            if let Some(root) = std::env::var_os("NEXA_COMPUTER_USE_EVIDENCE_DIR") {
                let root = std::path::PathBuf::from(root);
                std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
                baseline
                    .image
                    .save(root.join("capture-primary.png"))
                    .map_err(|error| error.to_string())?;
                fallback
                    .image
                    .save(root.join("capture-fallback.png"))
                    .map_err(|error| error.to_string())?;
                std::fs::write(
                    root.join("capture-fallback.json"),
                    serde_json::to_vec_pretty(&serde_json::json!({
                        "windowId": target.id, "width": target.width, "height": target.height,
                        "baselineBackend": baseline.backend, "fallbackBackend": fallback.backend,
                        "fallbackElapsedMs": fallback_elapsed_ms, "inputSent": false,
                        "startupFailureInjected": true,
                        "physicalPixelsConsistent": consistent,
                        "boundedStartupFallbackMs": startup_wait_ms,
                    }))
                    .unwrap(),
                )
                .map_err(|error| error.to_string())?;
            }
            if !consistent {
                return Err("Fallback pixels do not match the native physical frame; reject DPI padding or scaling drift.".into());
            }
            use windows::Win32::Foundation::{LPARAM, WPARAM};
            use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_APP};
            if unsafe {
                SendMessageW(
                    hwnd(target.id),
                    WM_APP + 98,
                    Some(WPARAM(0x11)),
                    Some(LPARAM(0)),
                )
            }
            .0 != 1
            {
                return Err("Fixture could not enable its capture-exclusion policy".into());
            }
            let protected =
                print_window(&target).expect_err("Fallback must respect capture exclusion");
            if !protected.to_string().contains("excludes capture") {
                return Err(protected.to_string());
            }
            Ok(())
        })();
        let _ = child.kill();
        let _ = child.wait();
        result.unwrap();
    }
}

fn print_window(expected: &WindowSnapshot) -> Result<RgbaImage, CoreError> {
    let _physical_pixels = PhysicalPixels::enter()?;
    let (_, current) = current_window_for_access(expected, true)?;
    ensure_visible(&current)?;
    if !same_capture_surface(expected, &current) {
        return Err(invalid("Window changed before fallback capture."));
    }
    let window = hwnd(current.id);
    let check_affinity = || -> Result<(), CoreError> {
        let mut affinity = 0_u32;
        unsafe { GetWindowDisplayAffinity(window, &mut affinity) }
            .map_err(|error| platform_error("check window capture permission", error))?;
        if affinity != 0 {
            return Err(invalid(
                "This window excludes capture; fallback is unavailable.",
            ));
        }
        Ok(())
    };
    check_affinity()?;
    let mut rect = RECT::default();
    unsafe { GetWindowRect(window, &mut rect) }
        .map_err(|error| platform_error("read fallback bounds", error))?;
    let (width, height) = (
        i64::from(rect.right) - i64::from(rect.left),
        i64::from(rect.bottom) - i64::from(rect.top),
    );
    if width <= 0 || height <= 0 || width * height > MAX_NATIVE_CAPTURE_PIXELS as i64 {
        return Err(invalid(
            "Fallback capture dimensions exceed the native pixel budget.",
        ));
    }
    let (width, height) = (width as u32, height as u32);
    // PrintWindow renders in the target's own logical coordinate space. A DPI-
    // unaware app on a scaled monitor otherwise paints only the top-left of a
    // physical-sized bitmap. Capture that exact source size, then map the full
    // frame to physical pixels before cropping the compositor's visible bounds.
    let target_dpi = unsafe { windows::Win32::UI::HiDpi::GetWindowDpiAwarenessContext(window) };
    if target_dpi.0.is_null() {
        return Err(invalid("Window DPI context is unavailable."));
    }
    let render_rect = {
        let previous =
            unsafe { windows::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(target_dpi) };
        if previous.0.is_null() {
            return Err(platform_error(
                "enter target capture coordinates",
                windows::core::Error::from_thread(),
            ));
        }
        let _restore = PhysicalPixels(previous);
        let mut logical = RECT::default();
        unsafe { GetWindowRect(window, &mut logical) }
            .map_err(|error| platform_error("read target rendering bounds", error))?;
        logical
    };
    let (render_width, render_height) = (
        i64::from(render_rect.right) - i64::from(render_rect.left),
        i64::from(render_rect.bottom) - i64::from(render_rect.top),
    );
    if render_width <= 0
        || render_height <= 0
        || render_width * render_height > MAX_NATIVE_CAPTURE_PIXELS as i64
    {
        return Err(invalid(
            "Target rendering dimensions exceed the native pixel budget.",
        ));
    }
    let (render_width, render_height) = (render_width as u32, render_height as u32);
    let x = i64::from(current.x) - i64::from(rect.left);
    let y = i64::from(current.y) - i64::from(rect.top);
    if x < 0
        || y < 0
        || x + i64::from(current.width) > i64::from(width)
        || y + i64::from(current.height) > i64::from(height)
    {
        return Err(invalid(
            "Fallback capture cannot map the visible physical frame exactly.",
        ));
    }
    let mut frame = GdiFrame {
        window,
        source: unsafe { GetWindowDC(Some(window)) },
        memory: HDC::default(),
        bitmap: HBITMAP::default(),
        original: HGDIOBJ::default(),
    };
    if frame.source.0.is_null() {
        return Err(platform_error(
            "get window drawing context",
            windows::core::Error::from_thread(),
        ));
    }
    frame.memory = unsafe { CreateCompatibleDC(Some(frame.source)) };
    if frame.memory.0.is_null() {
        return Err(platform_error(
            "create capture drawing context",
            windows::core::Error::from_thread(),
        ));
    }
    frame.bitmap =
        unsafe { CreateCompatibleBitmap(frame.source, render_width as i32, render_height as i32) };
    if frame.bitmap.0.is_null() {
        return Err(platform_error(
            "create capture bitmap",
            windows::core::Error::from_thread(),
        ));
    }
    frame.original = unsafe { SelectObject(frame.memory, frame.bitmap.into()) };
    if frame.original.0.is_null() || frame.original.0 as isize == -1 {
        return Err(invalid("Could not select the capture bitmap."));
    }
    unsafe {
        PatBlt(
            frame.memory,
            0,
            0,
            render_width as i32,
            render_height as i32,
            BLACKNESS,
        )
    }
    .ok()
    .map_err(|error| platform_error("initialize capture bitmap", error))?;
    let rendered = {
        let previous =
            unsafe { windows::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(target_dpi) };
        if previous.0.is_null() {
            return Err(platform_error(
                "enter target rendering context",
                windows::core::Error::from_thread(),
            ));
        }
        let _restore = PhysicalPixels(previous);
        unsafe {
            PrintWindow(
                window,
                frame.memory,
                PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT),
            )
            .as_bool()
        }
    };
    if !rendered {
        return Err(invalid("The window does not support PrintWindow capture."));
    }
    unsafe {
        SelectObject(frame.memory, frame.original);
    }
    frame.original = HGDIOBJ::default();
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: render_width as i32,
            biHeight: -(render_height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bytes = vec![0_u8; render_width as usize * render_height as usize * 4];
    if unsafe {
        GetDIBits(
            frame.memory,
            frame.bitmap,
            0,
            render_height,
            Some(bytes.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        )
    } != render_height as i32
    {
        return Err(invalid("The fallback returned an incomplete bitmap."));
    }
    check_affinity()?;
    let (_, after) = current_window_for_access(&current, true)?;
    ensure_visible(&after)?;
    if !same_capture_surface(&current, &after) {
        return Err(invalid("Window changed during fallback capture."));
    }
    if bytes.chunks_exact(4).all(|pixel| pixel[..3] == [0, 0, 0]) {
        return Err(invalid(
            "The fallback returned a blank frame; no observation was accepted.",
        ));
    }
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    let image = RgbaImage::from_raw(render_width, render_height, bytes)
        .ok_or_else(|| invalid("Invalid fallback bitmap dimensions."))?;
    let image = if (render_width, render_height) == (width, height) {
        image
    } else {
        image::imageops::resize(&image, width, height, FilterType::Triangle)
    };
    Ok(
        image::imageops::crop_imm(&image, x as u32, y as u32, current.width, current.height)
            .to_image(),
    )
}
