//! Explicit monitor capture for WebView hosts whose picker only lists windows.
//! Frames are scaled and encoded in memory, never saved to disk.

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopMonitor {
    id: String,
    width: i32,
    height: i32,
    primary: bool,
    #[serde(skip)]
    #[cfg(windows)]
    x: i32,
    #[serde(skip)]
    #[cfg(windows)]
    y: i32,
}

#[tauri::command]
pub async fn list_desktop_monitors_cmd() -> Result<Vec<DesktopMonitor>, String> {
    #[cfg(windows)]
    return tokio::task::spawn_blocking(platform::monitors)
        .await
        .map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    Ok(Vec::new())
}

#[tauri::command]
pub async fn list_desktop_windows_cmd() -> Result<serde_json::Value, String> {
    let windows =
        tokio::task::spawn_blocking(nexa_core::tools::computer_use_tool::list_user_share_windows)
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({"supported": cfg!(windows), "windows": windows}))
}

#[tauri::command]
pub async fn capture_desktop_window_cmd(window_id: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        nexa_core::tools::computer_use_tool::capture_user_share_window(&window_id)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn capture_desktop_monitor_cmd(monitor_id: String) -> Result<String, String> {
    #[cfg(windows)]
    return tokio::task::spawn_blocking(move || platform::capture(&monitor_id))
        .await
        .map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    {
        let _ = monitor_id;
        Err("Use the system screen picker on this platform".into())
    }
}

#[cfg(windows)]
mod platform {
    use super::DesktopMonitor;
    use base64::Engine;
    use windows::core::{BOOL, PCWSTR};
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::HiDpi::{
        SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };

    struct PhysicalPixels(DPI_AWARENESS_CONTEXT);
    impl PhysicalPixels {
        fn enter() -> Self {
            Self(unsafe {
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
            })
        }
    }
    impl Drop for PhysicalPixels {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                unsafe {
                    SetThreadDpiAwarenessContext(self.0);
                }
            }
        }
    }

    pub fn monitors() -> Result<Vec<DesktopMonitor>, String> {
        let _pixels = PhysicalPixels::enter();
        unsafe extern "system" fn collect(
            handle: HMONITOR,
            _: HDC,
            _: *mut RECT,
            data: LPARAM,
        ) -> BOOL {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo) }.as_bool() {
                let rect = info.monitorInfo.rcMonitor;
                let length = info
                    .szDevice
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(info.szDevice.len());
                let name = String::from_utf16_lossy(&info.szDevice[..length]);
                let mut device = DISPLAY_DEVICEW {
                    cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
                    ..Default::default()
                };
                // DISPLAY1/DISPLAY2 can be reassigned after hotplug. Bind the
                // selected source to its monitor interface instead of its slot.
                let has_interface = unsafe {
                    EnumDisplayDevicesW(PCWSTR(info.szDevice.as_ptr()), 0, &mut device, 1)
                }
                .as_bool();
                let interface_len = device
                    .DeviceID
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(device.DeviceID.len());
                let id = if has_interface && interface_len > 0 {
                    String::from_utf16_lossy(&device.DeviceID[..interface_len])
                } else {
                    // Virtual displays may not have a physical interface.
                    format!(
                        "{name}:{handle:?}:{}:{}:{}:{}",
                        rect.left, rect.top, rect.right, rect.bottom
                    )
                };
                let list = unsafe { &mut *(data.0 as *mut Vec<DesktopMonitor>) };
                list.push(DesktopMonitor {
                    id,
                    width: rect.right - rect.left,
                    height: rect.bottom - rect.top,
                    x: rect.left,
                    y: rect.top,
                    primary: info.monitorInfo.dwFlags & 1 != 0,
                });
            }
            true.into()
        }
        let mut result: Vec<DesktopMonitor> = Vec::new();
        if !unsafe {
            EnumDisplayMonitors(
                None,
                None,
                Some(collect),
                LPARAM(&mut result as *mut _ as isize),
            )
        }
        .as_bool()
        {
            return Err("Unable to enumerate desktop monitors".into());
        }
        result.sort_by_key(|m| (!m.primary, m.x, m.y));
        Ok(result)
    }

    struct CaptureResources {
        screen: HDC,
        memory: HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
    }
    impl Drop for CaptureResources {
        fn drop(&mut self) {
            unsafe {
                if !self.previous.is_invalid() {
                    SelectObject(self.memory, self.previous);
                }
                if !self.bitmap.is_invalid() {
                    let _ = DeleteObject(self.bitmap.into());
                }
                if !self.memory.is_invalid() {
                    let _ = DeleteDC(self.memory);
                }
                if !self.screen.is_invalid() {
                    ReleaseDC(None, self.screen);
                }
            }
        }
    }

    pub fn capture(id: &str) -> Result<String, String> {
        let _pixels = PhysicalPixels::enter();
        // Re-enumerate instead of trusting caller-supplied coordinates or stale
        // handles. A disconnected monitor ends the capture, never switches it.
        let monitor = monitors()?
            .into_iter()
            .find(|m| m.id == id)
            .ok_or("The selected monitor is no longer connected")?;
        if monitor.width <= 0 || monitor.height <= 0 {
            return Err("Invalid monitor dimensions".into());
        }
        let scale = (1568.0 / f64::from(monitor.width.max(monitor.height))).min(1.0);
        let width = (f64::from(monitor.width) * scale).round().max(1.0) as i32;
        let height = (f64::from(monitor.height) * scale).round().max(1.0) as i32;
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        unsafe {
            let mut resources = CaptureResources {
                screen: GetDC(None),
                memory: HDC::default(),
                bitmap: HBITMAP::default(),
                previous: HGDIOBJ::default(),
            };
            if resources.screen.is_invalid() {
                return Err("Unable to access the desktop".into());
            }
            resources.memory = CreateCompatibleDC(Some(resources.screen));
            resources.bitmap = CreateCompatibleBitmap(resources.screen, width, height);
            if resources.memory.is_invalid() || resources.bitmap.is_invalid() {
                return Err("Unable to allocate desktop capture".into());
            }
            resources.previous = SelectObject(resources.memory, resources.bitmap.into());
            if resources.previous.is_invalid() {
                return Err("Unable to select desktop bitmap".into());
            }
            SetStretchBltMode(resources.memory, HALFTONE);
            let _ = SetBrushOrgEx(resources.memory, 0, 0, None);
            if !StretchBlt(
                resources.memory,
                0,
                0,
                width,
                height,
                Some(resources.screen),
                monitor.x,
                monitor.y,
                monitor.width,
                monitor.height,
                SRCCOPY | CAPTUREBLT,
            )
            .as_bool()
            {
                return Err("Unable to capture the selected monitor".into());
            }
            SelectObject(resources.memory, resources.previous);
            resources.previous = HGDIOBJ::default();
            let mut info = BITMAPINFO {
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
            if GetDIBits(
                resources.screen,
                resources.bitmap,
                0,
                height as u32,
                Some(pixels.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            ) != height
            {
                return Err("Unable to read desktop pixels".into());
            }
        }
        let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
        for bgra in pixels.chunks_exact(4) {
            rgb.extend_from_slice(&[bgra[2], bgra[1], bgra[0]]);
        }
        let mut encoded = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 70)
            .encode(
                &rgb,
                width as u32,
                height as u32,
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|e| e.to_string())?;
        Ok(base64::engine::general_purpose::STANDARD.encode(encoded))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        #[ignore = "Requires an interactive Windows desktop"]
        fn native_monitor_capture() {
            let displays = monitors().expect("enumerate monitors");
            assert!(!displays.is_empty());
            for display in &displays {
                let encoded = capture(&display.id).expect("capture selected display");
                let jpeg = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .unwrap();
                let frame = image::load_from_memory(&jpeg).expect("valid JPEG");
                assert!(frame.width() > 0 && frame.height() > 0);
                assert!(frame.width().max(frame.height()) <= 1568);
                let expected_ratio = display.width as f64 / display.height as f64;
                assert!(
                    (frame.width() as f64 / frame.height() as f64 - expected_ratio).abs() < 0.01
                );
                println!(
                    "Captured {}x{} monitor as {}x{} JPEG",
                    display.width,
                    display.height,
                    frame.width(),
                    frame.height()
                );
            }
            assert!(capture("disconnected-monitor")
                .unwrap_err()
                .contains("no longer connected"));
        }
    }
}
