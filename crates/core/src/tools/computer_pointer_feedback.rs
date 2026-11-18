//! The native stop indicator and monitor edges for Computer Use.

use crate::agent_pointer::accent;
pub(super) use crate::agent_pointer::reduced_motion;
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SetWindowPos, UpdateLayeredWindow, HWND_TOPMOST,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, ULW_ALPHA, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

#[derive(Default)]
struct State {
    active: bool,
    scale: f64,
    status_placement: Option<StatusPlacement>,
}

pub(super) fn configure_appearance(accent: [u8; 3], reduced_motion: bool) {
    crate::agent_pointer::configure(accent, reduced_motion);
}

static STATUS_WINDOW: AtomicUsize = AtomicUsize::new(0);

pub(super) fn register_status_window(window: u64) {
    STATUS_WINDOW.store(window as usize, Ordering::Release);
}

fn owned_status_window() -> Option<HWND> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowTextW, GetWindowThreadProcessId, IsWindow,
    };
    let handle = HWND(STATUS_WINDOW.load(Ordering::Acquire) as *mut _);
    if !unsafe { IsWindow(Some(handle)) }.as_bool() {
        return None;
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(handle, Some(&mut pid));
    }
    let mut title = [0_u16; 64];
    let length = unsafe { GetWindowTextW(handle, &mut title) } as usize;
    (pid == std::process::id() && String::from_utf16_lossy(&title[..length]) == "Nexa Computer Use")
        .then_some(handle)
}

struct StatusPlacement {
    handle: HWND,
    position: POINT,
}

impl Drop for StatusPlacement {
    fn drop(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::{SWP_NOSIZE, SWP_NOZORDER};
        if owned_status_window() == Some(self.handle) {
            let _ = unsafe {
                SetWindowPos(
                    self.handle,
                    None,
                    self.position.x,
                    self.position.y,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER,
                )
            };
        }
    }
}

/// Keep the Stop surface available while moving it out of an admitted pointer
/// target. Only the exact registered Nexa window can be repositioned; all other
/// occluding windows still fail the normal input ownership check.
pub(super) fn avoid_status_at(point: (i32, i32)) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, IsWindowVisible, SWP_NOSIZE, SWP_NOZORDER,
    };
    FEEDBACK.with(|state| {
        let mut state = state.borrow_mut();
        if !state.active {
            return;
        }
        let Some(handle) = owned_status_window() else {
            return;
        };
        if !unsafe { IsWindowVisible(handle) }.as_bool() {
            return;
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(handle, &mut rect) }.is_err()
            || point.0 < rect.left
            || point.0 >= rect.right
            || point.1 < rect.top
            || point.1 >= rect.bottom
        {
            return;
        }
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe {
            GetMonitorInfoW(
                MonitorFromWindow(handle, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            )
        }
        .as_bool()
        {
            return;
        }
        let area = monitor.rcWork;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let margin = (12.0 * state.scale).round() as i32;
        let x = area.left + ((area.right - area.left - width) / 2).max(0);
        let bottom = (area.bottom - height - margin).max(area.top);
        let y = if point.1 >= bottom {
            area.top + margin
        } else {
            bottom
        };
        if unsafe {
            SetWindowPos(
                handle,
                None,
                x,
                y,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER,
            )
        }
        .is_ok()
            && state.status_placement.is_none()
        {
            state.status_placement = Some(StatusPlacement {
                handle,
                position: POINT {
                    x: rect.left,
                    y: rect.top,
                },
            });
        }
    });
}

thread_local! { static FEEDBACK: RefCell<State> = RefCell::new(State::default()); }

#[derive(Default)]
struct FrameState {
    signature: Vec<(i32, i32, i32, i32, u32)>,
    accent: [u8; 3],
    surfaces: Vec<FrameSurface>,
}

thread_local! { static FRAMES: RefCell<FrameState> = RefCell::new(FrameState::default()); }

/// Called on the desktop main thread from the same projection as the Stop
/// surface. Four thin, click-through native surfaces leave the desktop center
/// untouched, without another full-screen WebView or a polling animation.
pub(super) fn set_control_targets(targets: &[u64]) {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    };
    let mut signature = Vec::new();
    for target in targets {
        let handle = HWND(*target as usize as *mut _);
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe {
            GetMonitorInfoW(
                MonitorFromWindow(handle, MONITOR_DEFAULTTOPRIMARY),
                &mut monitor,
            )
        }
        .as_bool()
        {
            let rect = monitor.rcMonitor;
            let dpi = unsafe { GetDpiForWindow(handle) }.max(96);
            signature.push((rect.left, rect.top, rect.right, rect.bottom, dpi));
        }
    }
    signature.sort_unstable();
    signature.dedup();
    let color = accent();
    FRAMES.with(|frames| {
        let mut frames = frames.borrow_mut();
        if frames.signature == signature && frames.accent == color {
            return;
        }
        frames.surfaces.clear();
        frames.signature = signature;
        frames.accent = color;
        for &(left, top, right, bottom, dpi) in &frames.signature.clone() {
            let width = right - left;
            let height = bottom - top;
            let thickness = (12.0 * f64::from(dpi) / 96.0).round() as i32;
            if width <= thickness * 2 || height <= thickness * 2 {
                continue;
            }
            for (x, y, w, h, edge) in [
                (left, top, width, thickness, 0),
                (left, bottom - thickness, width, thickness, 1),
                (left, top + thickness, thickness, height - thickness * 2, 2),
                (
                    right - thickness,
                    top + thickness,
                    thickness,
                    height - thickness * 2,
                    3,
                ),
            ] {
                let pixels = frame_pixels(w, h, edge, color);
                match FrameSurface::create_surface(w, h, (x, y), &pixels) {
                    Ok(surface) => frames.surfaces.push(surface),
                    Err(error) => {
                        tracing::warn!("Could not draw the Computer Use screen frame: {error}")
                    }
                }
            }
        }
    });
}

fn frame_pixels(width: i32, height: i32, edge: u8, accent: [u8; 3]) -> Vec<u8> {
    let mut pixels = vec![0; width as usize * height as usize * 4];
    for y in 0..height {
        for x in 0..width {
            let (distance, thickness) = match edge {
                0 => (y, height),
                1 => (height - 1 - y, height),
                2 => (x, width),
                _ => (width - 1 - x, width),
            };
            let fade = 1.0 - f64::from(distance) / f64::from((thickness - 1).max(1));
            let alpha = (205.0 * fade.powi(2)) as u32;
            let offset = (y as usize * width as usize + x as usize) * 4;
            for (channel, value) in accent.into_iter().rev().enumerate() {
                pixels[offset + channel] = (u32::from(value) * alpha / 255) as u8;
            }
            pixels[offset + 3] = alpha as u8;
        }
    }
    pixels
}

pub(super) struct StatusGuard(std::marker::PhantomData<std::rc::Rc<()>>);

impl StatusGuard {
    pub(super) fn begin(target: u64) -> Self {
        let dpi = unsafe { GetDpiForWindow(HWND(target as usize as *mut _)) }.max(96);
        FEEDBACK.with(|state| {
            *state.borrow_mut() = State {
                active: true,
                scale: f64::from(dpi) / 96.0,
                status_placement: None,
            }
        });
        Self(std::marker::PhantomData)
    }
}

impl Drop for StatusGuard {
    fn drop(&mut self) {
        FEEDBACK.with(|state| *state.borrow_mut() = State::default());
    }
}

struct FrameSurface {
    handle: HWND,
    width: i32,
    height: i32,
}

impl FrameSurface {
    fn create_surface(
        width: i32,
        height: i32,
        point: (i32, i32),
        pixels: &[u8],
    ) -> windows::core::Result<Self> {
        let handle = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST,
                w!("STATIC"),
                w!("Nexa monitor frame"),
                WS_POPUP,
                point.0,
                point.1,
                width,
                height,
                None,
                None,
                None,
                None,
            )
        }?;
        let window = Self {
            handle,
            width,
            height,
        };
        window.paint(point, pixels)?;
        Ok(window)
    }

    fn paint(&self, point: (i32, i32), pixels: &[u8]) -> windows::core::Result<()> {
        let Self {
            handle,
            width,
            height,
        } = *self;
        debug_assert_eq!(pixels.len(), width as usize * height as usize * 4);
        // The unowned tool window is outside the target's WGC surface.
        let screen = unsafe { GetDC(None) };
        if screen.is_invalid() {
            return Err(windows::core::Error::from_thread());
        }
        let memory = unsafe { CreateCompatibleDC(Some(screen)) };
        if memory.is_invalid() {
            unsafe {
                ReleaseDC(None, screen);
            }
            return Err(windows::core::Error::from_thread());
        }
        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = unsafe {
            CreateDIBSection(
                Some(screen),
                &bitmap_info,
                DIB_RGB_COLORS,
                &mut bits,
                None,
                0,
            )
        };
        let result = match bitmap {
            Ok(bitmap) => {
                let previous = unsafe { SelectObject(memory, bitmap.into()) };
                unsafe {
                    std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
                }
                let position = POINT {
                    x: point.0,
                    y: point.1,
                };
                let size = SIZE {
                    cx: width,
                    cy: height,
                };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                    BlendFlags: 0,
                };
                let result = unsafe {
                    UpdateLayeredWindow(
                        handle,
                        Some(screen),
                        Some(&position),
                        Some(&size),
                        Some(memory),
                        Some(&POINT::default()),
                        COLORREF(0),
                        Some(&blend),
                        ULW_ALPHA,
                    )
                };
                unsafe {
                    SelectObject(memory, previous);
                    let _ = DeleteObject(bitmap.into());
                }
                result
            }
            Err(error) => Err(error),
        };
        unsafe {
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
        }
        result?;
        unsafe {
            SetWindowPos(
                handle,
                Some(HWND_TOPMOST),
                point.0,
                point.1,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        }?;
        Ok(())
    }
}

impl Drop for FrameSurface {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screen_frame_fades_inward_from_every_edge_without_an_opaque_center() {
        for (width, height, edge, outer, inner) in [
            (120, 12, 0, 0, 120 * 11),
            (120, 12, 1, 120 * 11, 0),
            (12, 120, 2, 0, 11),
            (12, 120, 3, 11, 0),
        ] {
            let pixels = frame_pixels(width, height, edge, [20, 184, 166]);
            assert_eq!(pixels[outer * 4 + 3], 205);
            assert_eq!(pixels[inner * 4 + 3], 0);
            assert!(pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[..3].iter().all(|c| *c <= p[3])));
        }
    }

    #[test]
    #[ignore = "requires interactive Windows; draws only click-through screen edges, sends no input"]
    fn native_screen_frame_is_visible_click_through_and_released_with_control() {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, GetForegroundWindow, IsWindow, IsWindowVisible, WindowFromPoint,
        };
        let foreground = unsafe { GetForegroundWindow() };
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point) }.unwrap();
        let hit = unsafe { WindowFromPoint(point) };
        set_control_targets(&[foreground.0 as usize as u64]);
        let handles = FRAMES.with(|frames| {
            frames
                .borrow()
                .surfaces
                .iter()
                .map(|surface| surface.handle)
                .collect::<Vec<_>>()
        });
        assert_eq!(handles.len(), 4);
        assert!(handles
            .iter()
            .all(|handle| unsafe { IsWindowVisible(*handle) }.as_bool()));
        assert_eq!(unsafe { GetForegroundWindow() }, foreground);
        assert_eq!(unsafe { WindowFromPoint(point) }, hit);
        set_control_targets(&[]);
        assert!(handles
            .iter()
            .all(|handle| !unsafe { IsWindow(Some(*handle)) }.as_bool()));
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop; moves only an isolated test status window"]
    fn status_obstruction_moves_only_registered_feedback_and_restores_on_exit() {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::HiDpi::{
            SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, GetForegroundWindow, GetWindowRect,
        };
        let previous_dpi =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point) }.unwrap();
        let foreground = unsafe { GetForegroundWindow() };
        let handle = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                w!("STATIC"),
                w!("Nexa Computer Use"),
                WS_POPUP,
                point.x - 40,
                point.y - 20,
                200,
                76,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        let _status = FrameSurface {
            handle,
            width: 200,
            height: 76,
        };
        unsafe {
            SetWindowPos(
                handle,
                Some(HWND_TOPMOST),
                point.x - 40,
                point.y - 20,
                200,
                76,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        }
        .unwrap();
        let mut original = RECT::default();
        unsafe { GetWindowRect(handle, &mut original) }.unwrap();
        {
            let _feedback = StatusGuard::begin(foreground.0 as usize as u64);
            // Unregistered windows are never moved, even with the same title.
            avoid_status_at((point.x, point.y));
            let mut current = RECT::default();
            unsafe { GetWindowRect(handle, &mut current) }.unwrap();
            assert_eq!(current, original);
            register_status_window(handle.0 as usize as u64);
            avoid_status_at((point.x, point.y));
            unsafe { GetWindowRect(handle, &mut current) }.unwrap();
            assert!(
                point.x < current.left
                    || point.x >= current.right
                    || point.y < current.top
                    || point.y >= current.bottom
            );
            assert_eq!(unsafe { GetForegroundWindow() }, foreground);
        }
        let mut restored = RECT::default();
        unsafe { GetWindowRect(handle, &mut restored) }.unwrap();
        assert_eq!(restored, original);
        register_status_window(0);
        unsafe {
            SetThreadDpiAwarenessContext(previous_dpi);
        }
    }
}
