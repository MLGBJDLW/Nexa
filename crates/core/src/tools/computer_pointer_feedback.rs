//! Ephemeral agent pointer. It never replaces a system cursor, activates a
//! window, intercepts input, or outlives the native action worker that owns it.

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
    window: Option<PointerWindow>,
    drawing_failed: bool,
    status_placement: Option<StatusPlacement>,
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

pub(super) struct PointerFeedback(std::marker::PhantomData<std::rc::Rc<()>>);

impl PointerFeedback {
    pub(super) fn begin(target: u64) -> Self {
        let dpi = unsafe { GetDpiForWindow(HWND(target as usize as *mut _)) }.max(96);
        FEEDBACK.with(|state| {
            *state.borrow_mut() = State {
                active: true,
                scale: f64::from(dpi) / 96.0,
                window: None,
                drawing_failed: false,
                status_placement: None,
            }
        });
        Self(std::marker::PhantomData)
    }
}

impl Drop for PointerFeedback {
    fn drop(&mut self) {
        FEEDBACK.with(|state| *state.borrow_mut() = State::default());
    }
}

/// Show only the verified target of an admitted action. Failure to draw is
/// cosmetic and must never trigger input retries or change the action receipt.
pub(super) fn show_at(point: (i32, i32)) {
    FEEDBACK.with(|state| {
        let mut state = state.borrow_mut();
        if !state.active || state.drawing_failed {
            return;
        }
        if state.window.is_none() {
            match PointerWindow::create(state.scale, point) {
                Ok(window) => state.window = Some(window),
                Err(error) => {
                    state.drawing_failed = true;
                    tracing::warn!("Could not draw the Nexa agent pointer: {error}");
                }
            }
        } else if let Some(window) = &state.window {
            let _ = unsafe {
                SetWindowPos(
                    window.handle,
                    Some(HWND_TOPMOST),
                    point.0 - window.hotspot,
                    point.1 - window.hotspot,
                    window.width,
                    window.height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
            };
        }
    });
}

struct PointerWindow {
    handle: HWND,
    width: i32,
    height: i32,
    hotspot: i32,
}

impl PointerWindow {
    fn create(scale: f64, point: (i32, i32)) -> windows::core::Result<Self> {
        let (width, height, pixels) = pointer_pixels(scale);
        let hotspot = (8.0 * scale).round() as i32;
        let handle = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST,
                w!("STATIC"),
                w!("Nexa agent pointer"),
                WS_POPUP,
                point.0 - hotspot,
                point.1 - hotspot,
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
            hotspot,
        };
        // A separate unowned tool window is excluded from the target's WGC
        // surface. It remains visible to the user and ordinary screen recording.
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
                    x: point.0 - hotspot,
                    y: point.1 - hotspot,
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
                point.0 - hotspot,
                point.1 - hotspot,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        }?;
        Ok(window)
    }
}

impl Drop for PointerWindow {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.handle) };
    }
}

fn inside_polygon(x: f64, y: f64, points: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut previous = points[points.len() - 1];
    for &point in points {
        if (point.1 > y) != (previous.1 > y)
            && x < (previous.0 - point.0) * (y - point.1) / (previous.1 - point.1) + point.0
        {
            inside = !inside;
        }
        previous = point;
    }
    inside
}

/// A compact 20x25 logical-pixel arrow in premultiplied BGRA. Four-sample
/// antialiasing, a quiet target glow and a fine white edge retain contrast on
/// light and dark surfaces without covering nearby controls with a label.
fn pointer_pixels(scale: f64) -> (i32, i32, Vec<u8>) {
    let width = (36.0 * scale).ceil() as i32;
    let height = (40.0 * scale).ceil() as i32;
    let outer = [
        (8., 8.),
        (9.2, 29.),
        (14.8, 23.8),
        (19.1, 32.2),
        (22.9, 30.4),
        (18.7, 22.),
        (26.4, 21.4),
    ];
    let inner = [
        (9.3, 10.3),
        (10.3, 26.6),
        (15.2, 22.),
        (19.6, 30.7),
        (21.3, 29.9),
        (16.9, 21.),
        (23.4, 20.5),
    ];
    let mut pixels = vec![0; width as usize * height as usize * 4];
    for row in 0..height {
        for column in 0..width {
            let mut sum = [0_u32; 4];
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = (f64::from(column) + (f64::from(sx) + 0.5) / 4.0) / scale;
                    let y = (f64::from(row) + (f64::from(sy) + 0.5) / 4.0) / scale;
                    let distance = (x - 8.).hypot(y - 8.);
                    let glow = (1.0 - distance / 7.5).clamp(0., 1.).powi(2);
                    let mut color = [240, 123, 157, (48.0 * glow) as u32];
                    // A restrained one-pixel shadow keeps the white outline
                    // readable on pale windows; it never expands the hotspot.
                    if inside_polygon(x - 0.5, y - 1.0, &outer) {
                        color = [79, 38, 49, 70];
                    }
                    if inside_polygon(x, y, &outer) {
                        color = [255, 255, 255, 255];
                    }
                    if inside_polygon(x, y, &inner) {
                        let gradient = ((y - 10.) / 21.).clamp(0., 1.);
                        color = [
                            (248. - 37. * gradient) as u32,
                            (128. - 49. * gradient) as u32,
                            (164. - 52. * gradient) as u32,
                            255,
                        ];
                    }
                    for index in 0..3 {
                        sum[index] += color[index] * color[3] / 255;
                    }
                    sum[3] += color[3];
                }
            }
            let offset = (row as usize * width as usize + column as usize) * 4;
            for index in 0..4 {
                pixels[offset + index] = (sum[index] / 16) as u8;
            }
        }
    }
    (width, height, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pointer_pixels_have_transparency_and_scale_without_changing_the_hotspot() {
        for scale in [1.0, 1.5, 2.0] {
            let (width, height, pixels) = pointer_pixels(scale);
            assert_eq!(pixels.len(), width as usize * height as usize * 4);
            assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] == 0));
            assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] == 255));
            assert!(pixels
                .chunks_exact(4)
                .all(|pixel| pixel[..3].iter().all(|channel| *channel <= pixel[3])));
        }
        if let Some(path) = std::env::var_os("NEXA_POINTER_PREVIEW") {
            let (width, height, mut pixels) = pointer_pixels(2.0);
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
                if pixel[3] > 0 {
                    for index in 0..3 {
                        pixel[index] =
                            (u32::from(pixel[index]) * 255 / u32::from(pixel[3])).min(255) as u8;
                    }
                }
            }
            image::RgbaImage::from_raw(width as u32, height as u32, pixels)
                .unwrap()
                .save(path)
                .unwrap();
        }
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop; draws feedback without sending input"]
    fn native_pointer_is_click_through_nonactivating_and_worker_scoped() {
        use windows::Win32::UI::HiDpi::{
            SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, GetForegroundWindow, IsWindow, WindowFromPoint,
        };
        let previous_dpi =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point) }.unwrap();
        let foreground = unsafe { GetForegroundWindow() };
        let hit = unsafe { WindowFromPoint(point) };
        let handle;
        {
            let _feedback = PointerFeedback::begin(foreground.0 as usize as u64);
            show_at((point.x, point.y));
            handle = FEEDBACK.with(|state| {
                state
                    .borrow()
                    .window
                    .as_ref()
                    .expect("native overlay created")
                    .handle
            });
            assert!(unsafe { IsWindow(Some(handle)) }.as_bool());
            assert_eq!(unsafe { GetForegroundWindow() }, foreground);
            assert_eq!(unsafe { WindowFromPoint(point) }, hit);
        }
        assert!(!unsafe { IsWindow(Some(handle)) }.as_bool());
        unsafe {
            SetThreadDpiAwarenessContext(previous_dpi);
        }
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
        let _status = PointerWindow {
            handle,
            width: 200,
            height: 76,
            hotspot: 0,
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
            let _feedback = PointerFeedback::begin(foreground.0 as usize as u64);
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
