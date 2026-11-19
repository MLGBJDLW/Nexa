//! Windows display capture for the explicit share picker and desktop observation.
//! Source coordinates stay in physical pixels. Only the bounded output is allocated.

use super::UserShareMonitor;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Area {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl Area {
    fn frame_size(self) -> Result<(i32, i32), String> {
        if self.width <= 0 || self.height <= 0 {
            return Err("The desktop has no capturable display area".into());
        }
        let scale = (f64::from(crate::media::MAX_LLM_IMAGE_DIMENSION)
            / f64::from(self.width.max(self.height)))
        .min(1.0);
        Ok((
            (f64::from(self.width) * scale).round().max(1.0) as i32,
            (f64::from(self.height) * scale).round().max(1.0) as i32,
        ))
    }
}

struct Frame {
    width: i32,
    height: i32,
    jpeg: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Layout {
    area: Area,
    monitors: i32,
}

pub(super) struct DesktopFrame {
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) monitors: i32,
    pub(super) jpeg: Vec<u8>,
}

fn capture_virtual_screen(
    layout: impl Fn() -> Layout,
    capture: impl FnOnce(Area) -> Result<Frame, String>,
) -> Result<DesktopFrame, String> {
    let before = layout();
    if before.monitors <= 0 {
        return Err("No active display is available".into());
    }
    let frame = capture(before.area)?;
    if layout() != before {
        return Err(
            "The display layout changed during the screenshot. Request a fresh observation.".into(),
        );
    }
    Ok(DesktopFrame {
        monitors: before.monitors,
        width: frame.width,
        height: frame.height,
        jpeg: frame.jpeg,
    })
}

pub fn monitors() -> Result<Vec<UserShareMonitor>, String> {
    let _pixels = super::platform::PhysicalPixels::enter().map_err(|e| e.to_string())?;
    native::read_monitors()
}

pub fn capture_monitor(id: &str) -> Result<String, String> {
    Ok(read_selected_monitor(id)?.encoded())
}

impl Frame {
    fn encoded(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&self.jpeg)
    }
}

pub fn observe_monitor(id: &str) -> Result<serde_json::Value, String> {
    let frame = read_selected_monitor(id)?;
    Ok(serde_json::json!({
        "controlAvailable": false,
        "image": {"width": frame.width, "height": frame.height, "mimeType": "image/jpeg", "base64": frame.encoded()}
    }))
}

fn read_selected_monitor(id: &str) -> Result<Frame, String> {
    let _pixels = super::platform::PhysicalPixels::enter().map_err(|e| e.to_string())?;
    let selected_area = || -> Result<Area, String> {
        let monitor = native::read_monitors()?
            .into_iter()
            .find(|monitor| monitor.id == id)
            .ok_or("The selected monitor is no longer connected")?;
        Ok(Area {
            x: monitor.x,
            y: monitor.y,
            width: monitor.width,
            height: monitor.height,
        })
    };
    let area = selected_area()?;
    let frame = native::read_frame(area)?;
    if selected_area()? != area {
        return Err("The selected display moved during capture. Request a fresh frame.".into());
    }
    Ok(frame)
}

pub(super) fn capture_desktop() -> Result<DesktopFrame, String> {
    let _pixels = super::platform::PhysicalPixels::enter().map_err(|e| e.to_string())?;
    capture_virtual_screen(native::layout, native::read_frame)
}

mod native {
    use super::{Area, Frame, Layout, UserShareMonitor};
    use windows::core::{BOOL, PCWSTR};
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CMONITORS, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    pub(super) fn layout() -> Layout {
        // Virtual-screen metrics span all active displays, including those
        // left of or above the primary. The desktop window itself does not.
        unsafe {
            Layout {
                area: Area {
                    x: GetSystemMetrics(SM_XVIRTUALSCREEN),
                    y: GetSystemMetrics(SM_YVIRTUALSCREEN),
                    width: GetSystemMetrics(SM_CXVIRTUALSCREEN),
                    height: GetSystemMetrics(SM_CYVIRTUALSCREEN),
                },
                monitors: GetSystemMetrics(SM_CMONITORS),
            }
        }
    }
    pub(super) fn read_monitors() -> Result<Vec<UserShareMonitor>, String> {
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
                let list = unsafe { &mut *(data.0 as *mut Vec<UserShareMonitor>) };
                list.push(UserShareMonitor {
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
        let mut result: Vec<UserShareMonitor> = Vec::new();
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

    pub(super) fn read_frame(area: Area) -> Result<Frame, String> {
        let (width, height) = area.frame_size()?;
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
                area.x,
                area.y,
                area.width,
                area.height,
                SRCCOPY | CAPTUREBLT,
            )
            .as_bool()
            {
                return Err("Unable to capture the requested display area".into());
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
        super::encode_bgra(pixels, width, height)
    }
}

fn encode_bgra(mut pixels: Vec<u8>, width: i32, height: i32) -> Result<Frame, String> {
    // RGB occupies less space than BGRA, so a forward pass never overwrites
    // the next source pixel and the encoder can reuse the capture buffer.
    let pixel_count = pixels.len() / 4;
    for index in 0..pixel_count {
        let source = index * 4;
        let rgb = [pixels[source + 2], pixels[source + 1], pixels[source]];
        pixels[index * 3..][..3].copy_from_slice(&rgb);
    }
    pixels.truncate(pixel_count * 3);
    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 70)
        .encode(
            &pixels,
            width as u32,
            height as u32,
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| e.to_string())?;
    Ok(Frame {
        width,
        height,
        jpeg: encoded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_observation_covers_offset_displays_with_one_bounded_read_only_frame() {
        // Synthetic dual-display canvas, left display above and left of primary.
        let layout = Layout {
            area: Area {
                x: -1280,
                y: -240,
                width: 3200,
                height: 1320,
            },
            monitors: 2,
        };
        let captured = capture_virtual_screen(
            || layout,
            |area| {
                assert_eq!(
                    (area.x, area.y, area.width, area.height),
                    (-1280, -240, 3200, 1320)
                );
                let (width, height) = area.frame_size()?;
                let mut pixels = Vec::new();
                for _ in 0..height {
                    for x in 0..width {
                        let color = if x * 3200 / width < 1280 {
                            [0, 0, 255, 0]
                        } else {
                            [255, 0, 0, 0]
                        };
                        pixels.extend_from_slice(&color);
                    }
                }
                encode_bgra(pixels, width, height)
            },
        )
        .unwrap();
        let image = image::load_from_memory(&captured.jpeg).unwrap().to_rgb8();
        assert_eq!((image.width(), image.height()), (1568, 647));
        assert!(image.get_pixel(20, 300).0[0] > 230);
        assert!(image.get_pixel(1500, 300).0[2] > 230);

        let output = super::super::desktop_frame_output(captured);
        assert_eq!(output.attachments.len(), 1);
        assert_eq!(output.attachments[0].mime_type, "image/jpeg");
        let data = output.data.unwrap();
        assert_eq!(data["target"]["kind"], "desktop");
        assert_eq!(data["monitorCount"], 2);
        assert_eq!(data["observationOnly"], true);
        assert!(data.get("observationId").is_none() && data.get("windowId").is_none());
    }

    #[test]
    fn desktop_observation_discards_a_frame_if_the_display_layout_changes() {
        let original = Layout {
            area: Area {
                x: -100,
                y: 0,
                width: 200,
                height: 100,
            },
            monitors: 2,
        };
        let calls = std::cell::Cell::new(0);
        let result = capture_virtual_screen(
            || {
                let first = calls.replace(calls.get() + 1) == 0;
                if first {
                    original
                } else {
                    Layout {
                        monitors: 1,
                        ..original
                    }
                }
            },
            |_| {
                Ok(Frame {
                    width: 200,
                    height: 100,
                    jpeg: vec![],
                })
            },
        );
        assert!(result.err().unwrap().contains("layout changed"));
        assert!(capture_virtual_screen(
            || Layout {
                monitors: 0,
                ..original
            },
            |_| panic!("no display")
        )
        .is_err());
        assert!(Area {
            width: 0,
            ..original.area
        }
        .frame_size()
        .is_err());
    }
}
