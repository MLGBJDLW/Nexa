use super::{permits_semantic_click, ControlDelivery};
use std::ffi::c_void;
use std::io::Cursor;
use std::sync::mpsc::{self, SyncSender};
use std::sync::OnceLock;
use std::thread;
use std::time::Instant;

use image::{imageops::FilterType, DynamicImage, ImageFormat, RgbaImage};
use windows::core::{Interface, BSTR, PWSTR};
use windows::Win32::{
    Foundation::{CloseHandle, FILETIME, HWND, RECT},
    System::{
        Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        },
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{
            GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
    UI::{
        Accessibility::{
            CUIAutomation, ExpandCollapseState_Collapsed, ExpandCollapseState_Expanded,
            ExpandCollapseState_LeafNode, ExpandCollapseState_PartiallyExpanded, IUIAutomation,
            IUIAutomation2, IUIAutomationCacheRequest, IUIAutomationElement,
            IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern,
            IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern,
            IUIAutomationValuePattern, ToggleState_Indeterminate, ToggleState_Off, ToggleState_On,
            TreeScope_Descendants, TreeScope_Element, UIA_AutomationIdPropertyId,
            UIA_BoundingRectanglePropertyId, UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId,
            UIA_ComboBoxControlTypeId, UIA_ControlTypePropertyId, UIA_DataItemControlTypeId,
            UIA_DocumentControlTypeId, UIA_EditControlTypeId,
            UIA_ExpandCollapseExpandCollapseStatePropertyId, UIA_ExpandCollapsePatternId,
            UIA_HasKeyboardFocusPropertyId, UIA_HyperlinkControlTypeId, UIA_InvokePatternId,
            UIA_IsEnabledPropertyId, UIA_IsKeyboardFocusablePropertyId, UIA_IsOffscreenPropertyId,
            UIA_IsPasswordPropertyId, UIA_ListItemControlTypeId, UIA_MenuItemControlTypeId,
            UIA_NamePropertyId, UIA_PaneControlTypeId, UIA_RadioButtonControlTypeId,
            UIA_SelectionItemIsSelectedPropertyId, UIA_SelectionItemPatternId,
            UIA_SliderControlTypeId, UIA_SpinnerControlTypeId, UIA_TabItemControlTypeId,
            UIA_TextControlTypeId, UIA_TogglePatternId, UIA_ToggleToggleStatePropertyId,
            UIA_TreeItemControlTypeId, UIA_ValueIsReadOnlyPropertyId, UIA_ValuePatternId,
            UIA_ValueValuePropertyId, UIA_WindowControlTypeId, UIA_CONTROLTYPE_ID,
        },
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, IsWindowEnabled, SendInput, VkKeyScanW, INPUT, INPUT_0,
            INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
            KEYEVENTF_UNICODE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
            MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN,
            MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
            VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_HOME, VK_LBUTTON,
            VK_LEFT, VK_LWIN, VK_MBUTTON, VK_MENU, VK_NEXT, VK_PRIOR, VK_RBUTTON, VK_RETURN,
            VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
        },
        WindowsAndMessaging::{
            GetAncestor, GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindow,
            GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, IsZoomed,
            SetForegroundWindow, ShowWindow, WindowFromPoint, GA_ROOT, GW_OWNER, SW_RESTORE,
            WHEEL_DELTA,
        },
    },
};
use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings as CaptureSettings,
};
use windows_capture::window::Window;

use super::{
    before_control_commit, before_control_commit_as, screenshot_difference, screenshot_guard,
    screenshot_guard_patch_matches, screenshot_signature, screenshot_signatures_match, CaptureMode,
    CaptureOptions, CapturedWindow, ControlAction, ControlArgs, ControlCommitTracker,
    ControlFailure, ControlOutcome, CoordinateSpace, CoreError, ElementBounds, ModalOwnerHandoff,
    ObservedWindow, PreCommitFailureKind, UiElementSnapshot, UiElementState, UiElementValue,
    VisualVerification, WindowSnapshot,
};

// Coordinate-bearing screenshots must already fit the same pixel envelope
// used by the model image normalizer. The dispatcher may re-encode a noisy
// PNG to bounded JPEG, but it must never resize pixels after observation
// metadata and the single-use control token have been published.
const MAX_CAPTURE_EDGE: u32 = crate::media::MAX_LLM_IMAGE_DIMENSION;
const MAX_NATIVE_CAPTURE_PIXELS: u64 = 16_777_216;
const MAX_SCREENSHOT_PNG_BYTES: usize = 12 * 1024 * 1024;
const INPUT_SETTLE: Duration = Duration::from_millis(70);
const POST_ACTION_STATE_BUDGET: Duration = Duration::from_millis(1_200);
const POST_ACTION_SAMPLE_INTERVAL: Duration = Duration::from_millis(50);

use std::time::Duration;

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::InvalidInput(message.into())
}

fn platform_error(context: &str, error: impl std::fmt::Display) -> CoreError {
    CoreError::Internal(format!("{context}: {error}"))
}

/// WGC and UI Automation report physical pixels. Pool threads can inherit
/// a DPI-unaware host/test context, so scope every native input/capture to
/// physical coordinates and restore the previous context on all exits.
pub(super) struct PhysicalPixels(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT);

impl PhysicalPixels {
    pub(super) fn enter() -> Result<Self, CoreError> {
        use windows::Win32::UI::HiDpi::{
            SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous.0.is_null() {
            return Err(platform_error(
                "enter physical desktop coordinates",
                windows::core::Error::from_thread(),
            ));
        }
        Ok(Self(previous))
    }
}

impl Drop for PhysicalPixels {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(self.0);
        }
    }
}

fn process_identity(pid: u32) -> Result<(u64, String, String, u32), CoreError> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|error| platform_error("open target process for identity", error))?;
    let result = (|| {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }
            .map_err(|error| platform_error("read target process creation time", error))?;
        let process_started_at_100ns =
            (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);

        let mut path_buffer = vec![0_u16; 32_768];
        let mut path_len = path_buffer.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(path_buffer.as_mut_ptr()),
                &mut path_len,
            )
        }
        .map_err(|error| platform_error("read target executable identity", error))?;
        let executable_path = String::from_utf16_lossy(&path_buffer[..path_len as usize]);
        let executable_name = std::path::Path::new(executable_path.trim())
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| invalid("Target executable identity has no safe file name."))?
            .to_string();
        let executable_path_hash =
            blake3::hash(executable_path.trim().to_ascii_lowercase().as_bytes())
                .to_hex()
                .to_string();

        let mut session_id = 0_u32;
        unsafe { ProcessIdToSessionId(pid, &mut session_id) }
            .map_err(|error| platform_error("read target process session", error))?;
        Ok((
            process_started_at_100ns,
            executable_path_hash,
            executable_name,
            session_id,
        ))
    })();
    let _ = unsafe { CloseHandle(process) };
    result
}

fn host_executable_hash() -> Result<&'static str, CoreError> {
    static HASH: OnceLock<String> = OnceLock::new();
    if let Some(hash) = HASH.get() {
        return Ok(hash.as_str());
    }
    let hash = process_identity(std::process::id())?.1;
    let _ = HASH.set(hash);
    HASH.get()
        .map(String::as_str)
        .ok_or_else(|| CoreError::Internal("Host executable identity is unavailable.".into()))
}

fn host_executable_name() -> Result<&'static str, CoreError> {
    static NAME: OnceLock<String> = OnceLock::new();
    if let Some(name) = NAME.get() {
        return Ok(name.as_str());
    }
    let name = process_identity(std::process::id())?.2;
    let _ = NAME.set(name);
    NAME.get()
        .map(String::as_str)
        .ok_or_else(|| CoreError::Internal("Host executable name is unavailable.".into()))
}

fn window_class(handle: HWND) -> Result<String, CoreError> {
    let mut buffer = vec![0_u16; 512];
    let length = unsafe { GetClassNameW(handle, &mut buffer) };
    if length <= 0 {
        return Err(platform_error(
            "read target window class",
            windows::core::Error::from_thread(),
        ));
    }
    Ok(String::from_utf16_lossy(&buffer[..length as usize]))
}

fn protected_system_process(app_name: &str) -> bool {
    matches!(
        app_name.trim().to_ascii_lowercase().as_str(),
        "consent"
            | "consent.exe"
            | "credentialuibroker"
            | "credentialuibroker.exe"
            | "lockapp"
            | "lockapp.exe"
            | "logonui"
            | "logonui.exe"
            | "lsass"
            | "lsass.exe"
            | "winlogon"
            | "winlogon.exe"
    )
}

fn snapshot(window: &Window) -> Result<WindowSnapshot, CoreError> {
    let _physical_pixels = PhysicalPixels::enter()?;
    let handle = window.as_raw_hwnd();
    // GetWindowRect includes invisible resize borders and may be DPI
    // virtualized. WGC captures the compositor's visible frame instead.
    let rect = if unsafe { IsIconic(HWND(handle)).as_bool() } {
        window
            .rect()
            .map_err(|error| platform_error("read minimized window bounds", error))?
    } else {
        use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
        let mut rect = RECT::default();
        unsafe {
            DwmGetWindowAttribute(
                HWND(handle),
                DWMWA_EXTENDED_FRAME_BOUNDS,
                (&mut rect as *mut RECT).cast(),
                std::mem::size_of::<RECT>() as u32,
            )
        }
        .map_err(|error| platform_error("read visible physical window bounds", error))?;
        rect
    };
    let mut native_pid = 0_u32;
    let thread_id = unsafe { GetWindowThreadProcessId(HWND(handle), Some(&mut native_pid)) };
    if thread_id == 0 || native_pid == 0 {
        return Err(platform_error(
            "read native window owner",
            windows::core::Error::from_thread(),
        ));
    }
    let reported_pid = window
        .process_id()
        .map_err(|error| platform_error("read window pid", error))?;
    if reported_pid != native_pid {
        return Err(invalid("Window owner changed during enumeration."));
    }
    let (process_started_at_100ns, executable_path_hash, executable_name, session_id) =
        process_identity(native_pid)?;
    Ok(WindowSnapshot {
        input_epoch: super::super::computer_input::window_epoch(handle as usize as u64),
        id: handle as usize as u64,
        pid: native_pid,
        process_started_at_100ns,
        executable_path_hash,
        window_class: window_class(HWND(handle))?,
        session_id,
        app_name: executable_name,
        title: window
            .title()
            .map_err(|error| platform_error("read window title", error))?,
        x: rect.left,
        y: rect.top,
        width: (rect.right - rect.left).max(0) as u32,
        height: (rect.bottom - rect.top).max(0) as u32,
        minimized: unsafe { IsIconic(HWND(handle)).as_bool() },
        maximized: unsafe { IsZoomed(HWND(handle)).as_bool() },
        focused: unsafe { GetForegroundWindow() == HWND(handle) },
        modal_owner: None,
    })
}

fn observation_time_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

/// An owned top-level window whose exact owner is disabled establishes the
/// native modal relationship. Class names and shared PIDs are not evidence.
fn observed_modal_owner(window: &WindowSnapshot) -> Option<Box<WindowSnapshot>> {
    let owner = unsafe { GetWindow(hwnd(window.id), GW_OWNER) }.ok()?;
    if owner.0.is_null() || owner == hwnd(window.id) || unsafe { IsWindowEnabled(owner) }.as_bool()
    {
        return None;
    }
    let owner_window = Window::from_raw_hwnd(owner.0);
    if !owner_window.is_valid() {
        return None;
    }
    let expected = snapshot(&owner_window).ok()?;
    if expected.session_id != window.session_id {
        return None;
    }
    // Apply the same host/credential-surface exclusion as ordinary access.
    let (_, owner) = current_window(&expected).ok()?;
    Some(Box::new(owner))
}

fn modal_owner_matches(
    expected: Option<&WindowSnapshot>,
    current: Option<&WindowSnapshot>,
) -> bool {
    match (expected, current) {
        (None, None) => true,
        (Some(expected), Some(current)) => {
            super::desktop_target_identity(expected) == super::desktop_target_identity(current)
                && expected.window_class == current.window_class
        }
        _ => false,
    }
}

fn recovered_modal_owner(observed: &WindowSnapshot) -> Option<ModalOwnerHandoff> {
    let expected = observed.modal_owner.as_deref()?;
    let (window, current) = current_window(expected).ok()?;
    if !window.is_valid()
        || current.minimized
        || !unsafe { IsWindowVisible(hwnd(current.id)) }.as_bool()
        || !unsafe { IsWindowEnabled(hwnd(current.id)) }.as_bool()
    {
        return None;
    }
    Some(ModalOwnerHandoff {
        owner: current,
        observation_not_before_ms: observation_time_ms(),
    })
}

fn enumerated_windows(allow_host: bool) -> Result<Vec<(Window, WindowSnapshot)>, CoreError> {
    let host_executable_hash = host_executable_hash()?;
    let host_executable_name = host_executable_name()?;
    let windows =
        Window::enumerate().map_err(|error| platform_error("enumerate windows", error))?;
    let mut result = Vec::new();
    for window in windows {
        let Ok(snapshot) = snapshot(&window) else {
            continue;
        };
        if snapshot.title.trim().is_empty() || snapshot.width == 0 || snapshot.height == 0 {
            continue;
        }
        if !allow_host
            && (snapshot.pid == std::process::id()
                || snapshot.executable_path_hash == host_executable_hash
                || snapshot.app_name.eq_ignore_ascii_case(host_executable_name))
        {
            continue;
        }
        if protected_system_process(&snapshot.app_name) {
            continue;
        }
        result.push((window, snapshot));
    }
    Ok(result)
}

pub(super) fn list_windows() -> Result<Vec<WindowSnapshot>, CoreError> {
    let mut windows = enumerated_windows(false)?
        .into_iter()
        .map(|(_, snapshot)| snapshot)
        .collect::<Vec<_>>();
    windows.sort_by(|left, right| {
        right
            .focused
            .cmp(&left.focused)
            .then_with(|| {
                left.app_name
                    .to_ascii_lowercase()
                    .cmp(&right.app_name.to_ascii_lowercase())
            })
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(windows)
}

fn current_window(expected: &WindowSnapshot) -> Result<(Window, WindowSnapshot), CoreError> {
    current_window_for_access(expected, false)
}

fn current_window_for_access(
    expected: &WindowSnapshot,
    allow_host: bool,
) -> Result<(Window, WindowSnapshot), CoreError> {
    let handle = hwnd(expected.id);
    if !unsafe { IsWindow(Some(handle)).as_bool() } {
        return Err(invalid(format!(
            "Observed window {} no longer exists.",
            expected.id
        )));
    }
    let window = Window::from_raw_hwnd(handle.0);
    let current = snapshot(&window)?;
    if current.pid != expected.pid
        || current.process_started_at_100ns != expected.process_started_at_100ns
        || current.executable_path_hash != expected.executable_path_hash
        || current.window_class != expected.window_class
        || current.session_id != expected.session_id
        || current.app_name != expected.app_name
    {
        return Err(invalid(format!(
            "Window {} changed owner since observation; refusing stale desktop access.",
            expected.id
        )));
    }
    if !allow_host
        && (current.pid == std::process::id()
            || current.executable_path_hash == host_executable_hash()?
            || current
                .app_name
                .eq_ignore_ascii_case(host_executable_name()?))
    {
        return Err(invalid(
            "Nexa windows and approval surfaces are protected from computer control.",
        ));
    }
    if protected_system_process(&current.app_name) {
        return Err(invalid(
            "Windows credential, consent, lock, and logon surfaces are protected from computer control.",
        ));
    }
    Ok((window, current))
}

pub(super) fn user_share_windows() -> Result<Vec<super::UserShareWindow>, CoreError> {
    use base64::Engine;
    let mut sources = enumerated_windows(true)?
        .into_iter()
        .filter(|(_, window)| !window.minimized)
        .take(100)
        .map(|(_, window)| {
            let encoded = serde_json::to_vec(&window)
                .map_err(|error| platform_error("encode window source", error))?;
            Ok(super::UserShareWindow {
                id: base64::engine::general_purpose::STANDARD.encode(encoded),
                title: window.title,
                app_name: window.app_name,
                width: window.width,
                height: window.height,
            })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    sources.sort_by(|left, right| {
        left.app_name
            .cmp(&right.app_name)
            .then_with(|| left.title.cmp(&right.title))
    });
    Ok(sources)
}

pub(super) fn capture_user_share_window(source_id: &str) -> Result<String, CoreError> {
    let _physical_pixels = PhysicalPixels::enter()?;
    use base64::Engine;
    if source_id.len() > 16_384 {
        return Err(invalid("Invalid window sharing source"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(source_id)
        .map_err(|error| platform_error("decode window sharing source", error))?;
    let expected: WindowSnapshot = serde_json::from_slice(&bytes)
        .map_err(|error| platform_error("decode window sharing identity", error))?;
    let (window, current) = current_window_for_access(&expected, true)?;
    if current.minimized {
        return Err(invalid(
            "The shared window is minimized; restore it and start sharing again.",
        ));
    }
    let image = capture_rgba(window)?;
    current_window_for_access(&expected, true)?;
    let image = DynamicImage::ImageRgba8(image)
        .thumbnail(MAX_CAPTURE_EDGE, MAX_CAPTURE_EDGE)
        .to_rgb8();
    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 70)
        .encode(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|error| platform_error("encode shared window frame", error))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(encoded))
}

fn resized_png(image: RgbaImage) -> Result<(Vec<u8>, u32, u32, u32, u32), CoreError> {
    let native_width = image.width();
    let native_height = image.height();
    let longest = native_width.max(native_height);
    let (image_width, image_height, image) = if longest > MAX_CAPTURE_EDGE {
        let scale = MAX_CAPTURE_EDGE as f64 / longest as f64;
        let width = ((native_width as f64 * scale).round() as u32).max(1);
        let height = ((native_height as f64 * scale).round() as u32).max(1);
        (
            width,
            height,
            image::imageops::resize(&image, width, height, FilterType::Triangle),
        )
    } else {
        (native_width, native_height, image)
    };
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|error| platform_error("encode window screenshot", error))?;
    let png = png.into_inner();
    if png.len() > MAX_SCREENSHOT_PNG_BYTES {
        return Err(invalid(format!(
            "Encoded screenshot is {} bytes, exceeding the {} byte limit.",
            png.len(),
            MAX_SCREENSHOT_PNG_BYTES
        )));
    }
    Ok((png, image_width, image_height, native_width, native_height))
}

fn same_capture_surface(left: &WindowSnapshot, right: &WindowSnapshot) -> bool {
    left.id == right.id
        && left.pid == right.pid
        && left.process_started_at_100ns == right.process_started_at_100ns
        && left.executable_path_hash == right.executable_path_hash
        && left.window_class == right.window_class
        && left.session_id == right.session_id
        && left.title == right.title
        && left.x == right.x
        && left.y == right.y
        && left.width == right.width
        && left.height == right.height
        && left.minimized == right.minimized
}

struct OneFrameCapture {
    sender: SyncSender<Result<RgbaImage, String>>,
}

impl GraphicsCaptureApiHandler for OneFrameCapture {
    type Flags = SyncSender<Result<RgbaImage, String>>;
    type Error = String;

    fn new(context: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            sender: context.flags,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let width = frame.width();
        let height = frame.height();
        let result = (|| {
            let pixels = u64::from(width)
                .checked_mul(u64::from(height))
                .ok_or_else(|| "capture dimensions overflowed".to_string())?;
            if pixels == 0 || pixels > MAX_NATIVE_CAPTURE_PIXELS {
                return Err(format!(
                    "capture frame {width}x{height} exceeds the bounded native pixel budget"
                ));
            }
            let buffer = frame
                .buffer()
                .map_err(|error| format!("read capture frame: {error}"))?;
            let mut compact = Vec::new();
            let pixels = buffer.as_nopadding_buffer(&mut compact).to_vec();
            RgbaImage::from_raw(width, height, pixels)
                .ok_or_else(|| "capture frame had an invalid RGBA buffer".to_string())
        })();
        let _ = self.sender.send(result);
        capture_control.stop();
        Ok(())
    }
}

fn capture_rgba(window: Window) -> Result<RgbaImage, CoreError> {
    use super::super::computer_capture_lifecycle::CaptureCleanupPool;
    static CLEANUP: OnceLock<Result<CaptureCleanupPool, String>> = OnceLock::new();
    let cleanup = CLEANUP
        .get_or_init(|| CaptureCleanupPool::new(2))
        .as_ref()
        .map_err(|error| invalid(error.clone()))?;
    let permit = cleanup.acquire().map_err(invalid)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let settings = CaptureSettings::new(
        window,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        sender,
    );
    let control = OneFrameCapture::start_free_threaded(settings)
        .map_err(|error| platform_error("start Windows Graphics Capture worker", error))?;
    match receiver.recv_timeout(Duration::from_secs(2)) {
        Ok(frame) => {
            // A buggy graphics-driver teardown must not retain the global
            // desktop-input permit forever after a frame was already
            // delivered. Join on a detached cleanup thread with a bounded
            // acknowledgement; the captured frame remains authoritative.
            if let Err(error) = cleanup.finish(
                permit,
                move || {
                    control
                        .wait()
                        .map_err(|error| format!("join Windows Graphics Capture worker: {error}"))
                },
                Duration::from_millis(750),
            ) {
                tracing::warn!("{error}");
            }
            frame.map_err(|error| platform_error("decode Windows capture frame", error))
        }
        Err(error) => {
            // stop() joins the driver's thread; running it inline would
            // turn a bounded frame timeout into an unbounded tool hang.
            if let Err(error) = cleanup.finish(
                permit,
                move || {
                    control.stop().map_err(|error| {
                        format!("stop timed-out Windows Graphics Capture worker: {error}")
                    })
                },
                Duration::from_millis(750),
            ) {
                tracing::warn!("{error}");
            }
            Err(platform_error("receive Windows capture frame", error))
        }
    }
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, CoreError> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|error| platform_error("initialize UI Automation COM apartment", error))?;
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn create_automation() -> Result<IUIAutomation, CoreError> {
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|error| platform_error("create Windows UI Automation client", error))?;
    if let Ok(timeouts) = automation.cast::<IUIAutomation2>() {
        unsafe {
            let _ = timeouts.SetConnectionTimeout(750);
            let _ = timeouts.SetTransactionTimeout(1_000);
        }
    }
    Ok(automation)
}

fn control_role(control_type: UIA_CONTROLTYPE_ID) -> &'static str {
    match control_type {
        value if value == UIA_ButtonControlTypeId => "button",
        value if value == UIA_CheckBoxControlTypeId => "checkbox",
        value if value == UIA_ComboBoxControlTypeId => "combobox",
        value if value == UIA_DataItemControlTypeId => "data_item",
        value if value == UIA_DocumentControlTypeId => "document",
        value if value == UIA_EditControlTypeId => "edit",
        value if value == UIA_HyperlinkControlTypeId => "link",
        value if value == UIA_ListItemControlTypeId => "list_item",
        value if value == UIA_MenuItemControlTypeId => "menu_item",
        value if value == UIA_PaneControlTypeId => "pane",
        value if value == UIA_RadioButtonControlTypeId => "radio_button",
        value if value == UIA_SliderControlTypeId => "slider",
        value if value == UIA_SpinnerControlTypeId => "spinner",
        value if value == UIA_TabItemControlTypeId => "tab_item",
        value if value == UIA_TextControlTypeId => "text",
        value if value == UIA_TreeItemControlTypeId => "tree_item",
        value if value == UIA_WindowControlTypeId => "window",
        _ => "control",
    }
}

fn element_actions(
    control_type: UIA_CONTROLTYPE_ID,
    enabled: bool,
    focusable: bool,
    password: bool,
    invokable: bool,
    read_only: Option<bool>,
) -> Vec<String> {
    if !enabled {
        return Vec::new();
    }
    let mut actions = Vec::new();
    if focusable
        || matches!(
            control_type,
            value if value == UIA_ButtonControlTypeId
                || value == UIA_CheckBoxControlTypeId
                || value == UIA_ComboBoxControlTypeId
                || value == UIA_DataItemControlTypeId
                || value == UIA_EditControlTypeId
                || value == UIA_HyperlinkControlTypeId
                || value == UIA_ListItemControlTypeId
                || value == UIA_MenuItemControlTypeId
                || value == UIA_RadioButtonControlTypeId
                || value == UIA_SliderControlTypeId
                || value == UIA_SpinnerControlTypeId
                || value == UIA_TabItemControlTypeId
                || value == UIA_TreeItemControlTypeId
        )
    {
        actions.push("click".to_string());
    }
    if invokable {
        actions.push("invoke".to_string());
    }
    if !password && read_only == Some(false) {
        actions.push("set_value".to_string());
    }
    actions
}

fn semantic_cache_request(
    automation: &IUIAutomation,
) -> Result<IUIAutomationCacheRequest, CoreError> {
    let request = unsafe { automation.CreateCacheRequest() }
        .map_err(|error| platform_error("create UI Automation cache request", error))?;
    for property in [
        UIA_AutomationIdPropertyId,
        UIA_BoundingRectanglePropertyId,
        UIA_ControlTypePropertyId,
        UIA_HasKeyboardFocusPropertyId,
        UIA_IsEnabledPropertyId,
        UIA_IsKeyboardFocusablePropertyId,
        UIA_IsOffscreenPropertyId,
        UIA_IsPasswordPropertyId,
        UIA_NamePropertyId,
        UIA_ValueIsReadOnlyPropertyId,
        UIA_ToggleToggleStatePropertyId,
        UIA_SelectionItemIsSelectedPropertyId,
        UIA_ExpandCollapseExpandCollapseStatePropertyId,
    ] {
        unsafe { request.AddProperty(property) }
            .map_err(|error| platform_error("configure UI Automation cache", error))?;
    }
    for pattern in [
        UIA_InvokePatternId,
        UIA_ValuePatternId,
        UIA_TogglePatternId,
        UIA_SelectionItemPatternId,
        UIA_ExpandCollapsePatternId,
    ] {
        unsafe { request.AddPattern(pattern) }
            .map_err(|error| platform_error("cache UI Automation patterns", error))?;
    }
    unsafe { request.SetTreeScope(TreeScope_Element) }
        .map_err(|error| platform_error("scope UI Automation cache", error))?;
    Ok(request)
}

fn value_cache_request(automation: &IUIAutomation) -> Result<IUIAutomationCacheRequest, CoreError> {
    let request = unsafe { automation.CreateCacheRequest() }
        .map_err(|error| platform_error("create UI Automation value cache", error))?;
    for property in [UIA_IsPasswordPropertyId, UIA_ValueValuePropertyId] {
        unsafe { request.AddProperty(property) }
            .map_err(|error| platform_error("configure UI Automation value cache", error))?;
    }
    unsafe { request.AddPattern(UIA_ValuePatternId) }
        .map_err(|error| platform_error("cache UI Automation value pattern", error))?;
    unsafe { request.SetTreeScope(TreeScope_Element) }
        .map_err(|error| platform_error("scope UI Automation value cache", error))?;
    Ok(request)
}

fn retain_element_value(
    element: &IUIAutomationElement,
    snapshot: &mut UiElementSnapshot,
    request: &IUIAutomationCacheRequest,
    remaining: &mut usize,
) {
    // Never request Value for a password or an unrecognized provider.
    // Values are fetched only for retained elements, after the bounded
    // metadata scan. The fresh cache includes IsPassword in the same read.
    if snapshot.password || snapshot.state.read_only.is_none() {
        return;
    }
    let Ok(current) = (unsafe { element.BuildUpdatedCache(request) }) else {
        return;
    };
    if unsafe { current.CachedIsPassword() }.map_or(true, |password| password.as_bool()) {
        snapshot.password = true;
        snapshot.name.clear();
        snapshot.actions.retain(|action| action != "set_value");
        return;
    }
    let Ok(pattern) =
        (unsafe { current.GetCachedPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) })
    else {
        return;
    };
    if let Ok(value) = unsafe { pattern.CachedValue() } {
        snapshot.state.value = Some(UiElementValue::bounded(&value.to_string(), remaining));
    }
}

fn truncate_text(value: BSTR, max_chars: usize) -> String {
    let value = value.to_string();
    if value.chars().count() <= max_chars {
        value
    } else {
        value.chars().take(max_chars).collect()
    }
}

fn clipped_element_bounds(
    rect: RECT,
    window: &WindowSnapshot,
    image_width: u32,
    image_height: u32,
) -> Option<(ElementBounds, ElementBounds)> {
    let window_right = i64::from(window.x) + i64::from(window.width);
    let window_bottom = i64::from(window.y) + i64::from(window.height);
    let left = i64::from(rect.left).max(i64::from(window.x));
    let top = i64::from(rect.top).max(i64::from(window.y));
    let right = i64::from(rect.right).min(window_right);
    let bottom = i64::from(rect.bottom).min(window_bottom);
    if right <= left || bottom <= top || window.width == 0 || window.height == 0 {
        return None;
    }
    let screen_bounds = ElementBounds {
        x: left as i32,
        y: top as i32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    };
    let scale_x = image_width as f64 / window.width as f64;
    let scale_y = image_height as f64 / window.height as f64;
    let image_bounds = ElementBounds {
        x: (((left - i64::from(window.x)) as f64 * scale_x).round() as i32)
            .clamp(0, image_width.saturating_sub(1) as i32),
        y: (((top - i64::from(window.y)) as f64 * scale_y).round() as i32)
            .clamp(0, image_height.saturating_sub(1) as i32),
        width: (((right - left) as f64 * scale_x).round() as u32)
            .max(1)
            .min(image_width),
        height: (((bottom - top) as f64 * scale_y).round() as u32)
            .max(1)
            .min(image_height),
    };
    Some((image_bounds, screen_bounds))
}

fn cached_element_snapshot(
    element: &IUIAutomationElement,
    window: &WindowSnapshot,
    image_width: u32,
    image_height: u32,
) -> Option<UiElementSnapshot> {
    let offscreen = unsafe { element.CachedIsOffscreen() }
        .ok()
        .is_some_and(|value| value.as_bool());
    if offscreen {
        return None;
    }
    let control_type = unsafe { element.CachedControlType() }.ok()?;
    let rect = unsafe { element.CachedBoundingRectangle() }.ok()?;
    let (bounds, screen_bounds) = clipped_element_bounds(rect, window, image_width, image_height)?;
    let mut name = unsafe { element.CachedName() }
        .map(|value| truncate_text(value, 256))
        .unwrap_or_default();
    let automation_id = unsafe { element.CachedAutomationId() }
        .map(|value| truncate_text(value, 128))
        .unwrap_or_default();
    let enabled = unsafe { element.CachedIsEnabled() }.ok()?.as_bool();
    let focused = unsafe { element.CachedHasKeyboardFocus() }
        .ok()
        .is_some_and(|value| value.as_bool());
    let keyboard_focusable = unsafe { element.CachedIsKeyboardFocusable() }
        .ok()
        .is_some_and(|value| value.as_bool());
    let password = unsafe { element.CachedIsPassword() }.ok()?.as_bool();
    if password {
        name.clear();
    }
    let read_only =
        unsafe { element.GetCachedPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) }
            .ok()
            .and_then(|pattern| unsafe { pattern.CachedIsReadOnly() }.ok())
            .map(|value| value.as_bool());
    let toggle_state =
        unsafe { element.GetCachedPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) }
            .ok()
            .and_then(|pattern| unsafe { pattern.CachedToggleState() }.ok())
            .and_then(|state| match state {
                value if value == ToggleState_Off => Some("off"),
                value if value == ToggleState_On => Some("on"),
                value if value == ToggleState_Indeterminate => Some("indeterminate"),
                _ => None,
            });
    let selected = unsafe {
        element.GetCachedPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
    }
    .ok()
    .and_then(|pattern| unsafe { pattern.CachedIsSelected() }.ok())
    .map(|value| value.as_bool());
    let expand_collapse_state = unsafe {
        element
            .GetCachedPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
    }
    .ok()
    .and_then(|pattern| unsafe { pattern.CachedExpandCollapseState() }.ok())
    .and_then(|state| match state {
        value if value == ExpandCollapseState_Collapsed => Some("collapsed"),
        value if value == ExpandCollapseState_Expanded => Some("expanded"),
        value if value == ExpandCollapseState_PartiallyExpanded => Some("partially_expanded"),
        value if value == ExpandCollapseState_LeafNode => Some("leaf"),
        _ => None,
    });
    let invokable =
        unsafe { element.GetCachedPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) }
            .is_ok()
            || toggle_state.is_some()
            || selected.is_some()
            || expand_collapse_state.is_some_and(|state| state != "leaf");
    let actions = element_actions(
        control_type,
        enabled,
        keyboard_focusable,
        password,
        invokable,
        read_only,
    );
    let interactive = !actions.is_empty();
    if !interactive && name.trim().is_empty() && read_only.is_none() {
        return None;
    }
    Some(UiElementSnapshot {
        id: String::new(),
        role: control_role(control_type).to_string(),
        name,
        automation_id,
        bounds,
        enabled,
        focused,
        keyboard_focusable,
        interactive,
        password,
        actions,
        state: UiElementState {
            value: None,
            read_only,
            toggle_state,
            selected,
            expand_collapse_state,
        },
        screen_bounds,
    })
}

fn collect_elements(
    window: &WindowSnapshot,
    image_width: u32,
    image_height: u32,
    max_elements: usize,
) -> Result<Vec<UiElementSnapshot>, CoreError> {
    let _apartment = ComApartment::initialize()?;
    let automation = create_automation()?;
    let request = semantic_cache_request(&automation)?;
    let root = unsafe { automation.ElementFromHandle(hwnd(window.id)) }
        .map_err(|error| platform_error("inspect target window with UI Automation", error))?;
    let condition = unsafe { automation.ControlViewCondition() }
        .map_err(|error| platform_error("create UI Automation control-view condition", error))?;
    let elements = unsafe { root.FindAllBuildCache(TreeScope_Descendants, &condition, &request) }
        .map_err(|error| platform_error("enumerate UI Automation controls", error))?;
    let length = unsafe { elements.Length() }
        .map_err(|error| platform_error("count UI Automation controls", error))?
        .max(0) as usize;
    let scan_limit = length.min(max_elements.saturating_mul(8).clamp(100, 2_000));
    let mut projected = Vec::new();
    for index in 0..scan_limit {
        let Ok(element) = (unsafe { elements.GetElement(index as i32) }) else {
            continue;
        };
        if let Some(snapshot) = cached_element_snapshot(&element, window, image_width, image_height)
        {
            projected.push((snapshot, element));
        }
    }
    projected.sort_by_key(|(element, _)| {
        (
            !element.interactive,
            element.bounds.y,
            element.bounds.x,
            element.role.clone(),
        )
    });
    projected.truncate(max_elements);
    let value_request = value_cache_request(&automation)?;
    let mut remaining_value_characters = 8_192;
    for (index, (snapshot, element)) in projected.iter_mut().enumerate() {
        snapshot.id = format!("e{}", index + 1);
        retain_element_value(
            element,
            snapshot,
            &value_request,
            &mut remaining_value_characters,
        );
    }
    Ok(projected
        .into_iter()
        .map(|(snapshot, _)| snapshot)
        .collect())
}

fn glyph_rows(character: char) -> Option<[u8; 7]> {
    match character {
        'E' => Some([
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ]),
        'e' => Some([
            0b00000, 0b00000, 0b01110, 0b10001, 0b11111, 0b10000, 0b01111,
        ]),
        '0' => Some([
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ]),
        '1' => Some([
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ]),
        '2' => Some([
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ]),
        '3' => Some([
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ]),
        '4' => Some([
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ]),
        '5' => Some([
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ]),
        '6' => Some([
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ]),
        '7' => Some([
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ]),
        '8' => Some([
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ]),
        '9' => Some([
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ]),
        _ => None,
    }
}

fn annotate_elements(png: &[u8], elements: &[UiElementSnapshot]) -> Result<Vec<u8>, CoreError> {
    let mut image = image::load_from_memory(png)
        .map_err(|error| platform_error("decode screenshot for set-of-marks", error))?
        .to_rgba8();
    let width = image.width();
    let height = image.height();
    let outline = image::Rgba([0, 235, 150, 255]);
    let label_background = image::Rgba([12, 20, 28, 235]);
    let label_foreground = image::Rgba([255, 255, 255, 255]);
    for element in elements.iter().filter(|element| element.interactive) {
        let left = element.bounds.x.max(0) as u32;
        let top = element.bounds.y.max(0) as u32;
        let right = left
            .saturating_add(element.bounds.width.saturating_sub(1))
            .min(width.saturating_sub(1));
        let bottom = top
            .saturating_add(element.bounds.height.saturating_sub(1))
            .min(height.saturating_sub(1));
        for thickness in 0..2_u32 {
            let x0 = left.saturating_add(thickness).min(right);
            let y0 = top.saturating_add(thickness).min(bottom);
            let x1 = right.saturating_sub(thickness).max(x0);
            let y1 = bottom.saturating_sub(thickness).max(y0);
            for x in x0..=x1 {
                image.put_pixel(x, y0, outline);
                image.put_pixel(x, y1, outline);
            }
            for y in y0..=y1 {
                image.put_pixel(x0, y, outline);
                image.put_pixel(x1, y, outline);
            }
        }
        let label = element.id.clone();
        let label_width = (label.chars().count() as u32 * 12 + 4).min(width);
        let label_height = 18_u32.min(height);
        let label_left = left.min(width.saturating_sub(label_width));
        let label_top = top.saturating_sub(label_height);
        for y in label_top..label_top.saturating_add(label_height).min(height) {
            for x in label_left..label_left.saturating_add(label_width).min(width) {
                image.put_pixel(x, y, label_background);
            }
        }
        for (character_index, character) in label.chars().enumerate() {
            let Some(rows) = glyph_rows(character) else {
                continue;
            };
            for (row_index, row) in rows.iter().enumerate() {
                for column in 0..5_u32 {
                    if row & (1 << (4 - column)) == 0 {
                        continue;
                    }
                    for dy in 0..2_u32 {
                        for dx in 0..2_u32 {
                            let x = label_left + 2 + character_index as u32 * 12 + column * 2 + dx;
                            let y = label_top + 2 + row_index as u32 * 2 + dy;
                            if x < width && y < height {
                                image.put_pixel(x, y, label_foreground);
                            }
                        }
                    }
                }
            }
        }
    }
    let mut annotated = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut annotated, ImageFormat::Png)
        .map_err(|error| platform_error("encode set-of-marks screenshot", error))?;
    Ok(annotated.into_inner())
}

pub(super) fn capture_window(
    expected: &WindowSnapshot,
    options: CaptureOptions,
) -> Result<CapturedWindow, CoreError> {
    let _physical_pixels = PhysicalPixels::enter()?;
    // Stamp the beginning, not completion: an older in-flight capture must
    // not become fresh evidence merely because its provider returned late.
    let observation_captured_at_ms = observation_time_ms();
    let (window, current) = current_window(expected)?;
    let modal_owner = observed_modal_owner(&current);
    if current.minimized {
        return Err(invalid(format!(
            "Window {} is minimized. Focus or restore it before capture.",
            current.id
        )));
    }
    let image = capture_rgba(window)?;
    let post_capture = current_window(&current)?.1;
    if !same_capture_surface(&current, &post_capture) {
        return Err(invalid(
            "Target identity, title, or geometry changed during screenshot capture. Capture again.",
        ));
    }
    let (png, image_width, image_height, native_image_width, native_image_height) =
        resized_png(image)?;
    let (elements, semantic_error) = if options.include_elements {
        match collect_elements(
            &post_capture,
            image_width,
            image_height,
            options.max_elements,
        ) {
            Ok(elements) => (elements, None),
            Err(error) => (Vec::new(), Some(error.to_string())),
        }
    } else {
        (Vec::new(), None)
    };
    let mut final_snapshot = current_window(&post_capture)?.1;
    if !same_capture_surface(&post_capture, &final_snapshot) {
        return Err(invalid(
            "Target identity, title, or geometry changed while collecting UI semantics. Capture again.",
        ));
    }
    if options.include_elements && semantic_error.is_none() {
        let (verification_window, verification_snapshot) = current_window(&final_snapshot)?;
        if !same_capture_surface(&final_snapshot, &verification_snapshot) {
            return Err(invalid(
                "Target changed before semantic observation verification. Capture again.",
            ));
        }
        let verification_image = capture_rgba(verification_window)?;
        let (verification_png, verification_width, verification_height, _, _) =
            resized_png(verification_image)?;
        let consistent = verification_width == image_width
            && verification_height == image_height
            && screenshot_signature(&png)
                .zip(screenshot_signature(&verification_png))
                .is_some_and(|(before, after)| screenshot_signatures_match(&before, &after));
        if !consistent {
            return Err(invalid(
                "Window pixels changed while collecting UI semantics. Capture again to avoid a mixed observation.",
            ));
        }
    }
    let annotated_png = if options.mode == CaptureMode::SetOfMarks {
        Some(annotate_elements(&png, &elements)?)
    } else {
        None
    };
    if !modal_owner_matches(
        modal_owner.as_deref(),
        observed_modal_owner(&final_snapshot).as_deref(),
    ) {
        return Err(invalid(
            "The modal owner relationship changed during capture. Observe again.",
        ));
    }
    if current.input_epoch != super::super::computer_input::window_epoch(current.id) {
        return Err(invalid(
            "The user interacted with this window during capture. Observe again.",
        ));
    }
    final_snapshot.input_epoch = current.input_epoch;
    final_snapshot.modal_owner = modal_owner;
    Ok(CapturedWindow {
        snapshot: final_snapshot,
        png,
        image_width,
        image_height,
        native_image_width,
        native_image_height,
        elements,
        semantic_enabled: options.include_elements,
        semantic_error,
        annotated_png,
        observation_captured_at_ms,
    })
}

struct LiveElement {
    automation: IUIAutomation,
    element: IUIAutomationElement,
    snapshot: UiElementSnapshot,
    _apartment: ComApartment,
}

struct ResolvedPointerTarget {
    point: (i32, i32),
    live: Option<LiveElement>,
}

fn semantic_identity_matches(expected: &UiElementSnapshot, current: &UiElementSnapshot) -> bool {
    if expected.role != current.role
        || expected.name != current.name
        || expected.password != current.password
        || expected.state.read_only != current.state.read_only
        || expected.state.toggle_state != current.state.toggle_state
        || expected.state.selected != current.state.selected
        || expected.state.expand_collapse_state != current.state.expand_collapse_state
    {
        return false;
    }
    if !expected.automation_id.is_empty() && expected.automation_id != current.automation_id {
        return false;
    }
    let tolerance = 8_i32;
    expected.screen_bounds.x.abs_diff(current.screen_bounds.x) <= tolerance as u32
        && expected.screen_bounds.y.abs_diff(current.screen_bounds.y) <= tolerance as u32
        && expected
            .screen_bounds
            .width
            .abs_diff(current.screen_bounds.width)
            <= tolerance as u32
        && expected
            .screen_bounds
            .height
            .abs_diff(current.screen_bounds.height)
            <= tolerance as u32
}

fn resolve_live_element(
    window: &WindowSnapshot,
    observed: &ObservedWindow,
    expected: &UiElementSnapshot,
) -> Result<LiveElement, CoreError> {
    let (image_width, image_height) = observed
        .image_width
        .zip(observed.image_height)
        .ok_or_else(|| invalid("Semantic actions require a captured-window observation."))?;
    let apartment = ComApartment::initialize()?;
    let automation = create_automation()?;
    let request = semantic_cache_request(&automation)?;
    let root = unsafe { automation.ElementFromHandle(hwnd(window.id)) }
        .map_err(|error| platform_error("open target window UI Automation root", error))?;
    let condition = unsafe { automation.ControlViewCondition() }
        .map_err(|error| platform_error("create UI Automation action condition", error))?;
    let elements = unsafe { root.FindAllBuildCache(TreeScope_Descendants, &condition, &request) }
        .map_err(|error| platform_error("resolve UI Automation target", error))?;
    let length = (unsafe { elements.Length() }
        .map_err(|error| platform_error("count UI Automation action targets", error))?)
    .clamp(0, 2_000);
    let mut matched = None;
    for index in 0..length {
        let Ok(element) = (unsafe { elements.GetElement(index) }) else {
            continue;
        };
        let Some(current) = cached_element_snapshot(&element, window, image_width, image_height)
        else {
            continue;
        };
        if semantic_identity_matches(expected, &current) {
            if !current.enabled {
                return Err(invalid(format!(
                    "Element {} is disabled; refusing semantic action.",
                    expected.id
                )));
            }
            if matched.is_some() {
                return Err(invalid(format!(
                    "Element {} is ambiguous because multiple live UI Automation controls match its observed identity. Capture again or use an explicit approved coordinate fallback.",
                    expected.id
                )));
            }
            matched = Some((element, current));
        }
    }
    let Some((element, mut snapshot)) = matched else {
        return Err(invalid(format!(
            "Element {} changed or disappeared since observation. Capture the window again.",
            expected.id
        )));
    };
    if let Some(value) = expected.state.value.as_ref() {
        let request = value_cache_request(&automation)?;
        retain_element_value(&element, &mut snapshot, &request, &mut 1_024);
        if snapshot.password
            || snapshot
                .state
                .value
                .as_ref()
                .map(|current| &current.fingerprint)
                != Some(&value.fingerprint)
        {
            return Err(invalid(format!(
                "Element {} value changed or became unreadable since observation. Capture again before input.",
                expected.id,
            )));
        }
    }
    Ok(LiveElement {
        automation,
        element,
        snapshot,
        _apartment: apartment,
    })
}

fn live_element_point(
    live: &LiveElement,
    window: &WindowSnapshot,
) -> Result<(i32, i32), CoreError> {
    let mut clickable = windows::Win32::Foundation::POINT::default();
    let available = unsafe { live.element.GetClickablePoint(&mut clickable) }
        .map_err(|error| platform_error("resolve exact UI Automation click point", error))?;
    if !available.as_bool() {
        return Err(invalid(format!(
            "Element {} has no verified clickable point; refusing to guess its center.",
            live.snapshot.id
        )));
    }
    let point = (clickable.x, clickable.y);
    let right = i64::from(window.x) + i64::from(window.width);
    let bottom = i64::from(window.y) + i64::from(window.height);
    if i64::from(point.0) < i64::from(window.x)
        || i64::from(point.1) < i64::from(window.y)
        || i64::from(point.0) >= right
        || i64::from(point.1) >= bottom
    {
        return Err(invalid(
            "The live UI Automation click point is outside the approved target window. Capture again.",
        ));
    }
    ensure_point_matches_live_element(live, point)?;
    Ok(point)
}

fn ensure_point_matches_live_element(
    live: &LiveElement,
    point: (i32, i32),
) -> Result<(), CoreError> {
    let hit = unsafe {
        live.automation
            .ElementFromPoint(windows::Win32::Foundation::POINT {
                x: point.0,
                y: point.1,
            })
    }
    .map_err(|error| platform_error("hit-test UI Automation target", error))?;
    let walker = unsafe { live.automation.ControlViewWalker() }
        .map_err(|error| platform_error("create UI Automation target walker", error))?;
    let mut candidate = Some(hit);
    for _ in 0..64 {
        let Some(element) = candidate else {
            break;
        };
        let matches = unsafe { live.automation.CompareElements(&element, &live.element) }
            .map_err(|error| platform_error("compare UI Automation hit target", error))?;
        if matches.as_bool() {
            return Ok(());
        }
        candidate = unsafe { walker.GetParentElement(&element) }.ok();
    }
    Err(invalid(format!(
        "The verified click point no longer belongs to element {} or one of its descendants; capture again.",
        live.snapshot.id
    )))
}

fn invoke_element(
    element: &IUIAutomationElement,
    window: &WindowSnapshot,
    commit_tracker: &ControlCommitTracker,
) -> Result<&'static str, ControlFailure> {
    // Checkboxes may advertise Invoke as well as Toggle. The semantic
    // toggle avoids the native Invoke proxy's activating click path.
    if let Ok(pattern) =
        unsafe { element.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) }
    {
        let before = before_control_commit(
            unsafe { pattern.CurrentToggleState() }
                .map_err(|error| platform_error("read UI Automation toggle state", error)),
        )?;
        let route = if let Some(button) = native_child_handle(element, window, "button") {
            native_checkbox_toggle(button, window, before.0, commit_tracker)?;
            "native_checkbox_notification"
        } else {
            commit_tracker.result(commit_tracker.mark())?;
            commit_tracker.result(
                unsafe { pattern.Toggle() }
                    .map_err(|error| platform_error("toggle UI Automation element", error)),
            )?;
            "toggle_pattern"
        };
        // Some providers enqueue Toggle and return before their UI thread
        // applies it. Read back the semantic state; never repeat the input.
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let state = commit_tracker.result(
                unsafe { pattern.CurrentToggleState() }
                    .map_err(|error| platform_error("verify UI Automation toggle state", error)),
            )?;
            if state != before {
                break;
            }
            if Instant::now() >= deadline {
                return Err(commit_tracker.failure(invalid("The checkbox did not report a changed state after Toggle. Capture again before retrying; the action may still be pending.")));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        return Ok(route);
    }
    if let Ok(pattern) =
        unsafe { element.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) }
    {
        commit_tracker.result(commit_tracker.mark())?;
        commit_tracker.result(
            unsafe { pattern.Invoke() }
                .map_err(|error| platform_error("invoke UI Automation element", error)),
        )?;
        return Ok("invoke_pattern");
    }
    if let Ok(pattern) = unsafe {
        element.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
    } {
        commit_tracker.result(commit_tracker.mark())?;
        commit_tracker.result(
            unsafe { pattern.Select() }
                .map_err(|error| platform_error("select UI Automation element", error)),
        )?;
        return Ok("selection_item_pattern");
    }
    if let Ok(pattern) = unsafe {
        element
            .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
    } {
        let state = before_control_commit(
            unsafe { pattern.CurrentExpandCollapseState() }
                .map_err(|error| platform_error("read expand/collapse state", error)),
        )?;
        if state == ExpandCollapseState_Expanded {
            commit_tracker.result(commit_tracker.mark())?;
            commit_tracker.result(
                unsafe { pattern.Collapse() }
                    .map_err(|error| platform_error("collapse UI Automation element", error)),
            )?;
            return Ok("collapse_pattern");
        }
        if state == ExpandCollapseState_Collapsed || state == ExpandCollapseState_PartiallyExpanded
        {
            commit_tracker.result(commit_tracker.mark())?;
            commit_tracker.result(
                unsafe { pattern.Expand() }
                    .map_err(|error| platform_error("expand UI Automation element", error)),
            )?;
            return Ok("expand_pattern");
        }
    }
    Err(ControlFailure::pre_commit(invalid(
        "Target does not expose an invokable UI Automation pattern. Capture again and use click as an approved fallback.",
    )))
}

fn supports_semantic_invoke(element: &IUIAutomationElement) -> bool {
    unsafe {
        element
            .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
            .is_ok()
            || element
                .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                    UIA_SelectionItemPatternId,
                )
                .is_ok()
            || element
                .GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
                .is_ok()
            || element
                .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                    UIA_ExpandCollapsePatternId,
                )
                .is_ok()
    }
}

fn native_child_handle(
    element: &IUIAutomationElement,
    window: &WindowSnapshot,
    expected_class: &str,
) -> Option<HWND> {
    unsafe { element.CurrentNativeWindowHandle() }
        .ok()
        .filter(|handle| !handle.0.is_null())
        .filter(|handle| {
            let mut pid = 0;
            unsafe { GetWindowThreadProcessId(*handle, Some(&mut pid)) };
            pid == window.pid
                && unsafe {
                    windows::Win32::UI::WindowsAndMessaging::IsChild(hwnd(window.id), *handle)
                }
                .as_bool()
        })
        .filter(|handle| {
            let mut class = [0_u16; 128];
            let length = unsafe { GetClassNameW(*handle, &mut class) };
            String::from_utf16_lossy(&class[..length.max(0) as usize])
                .eq_ignore_ascii_case(expected_class)
        })
}

fn set_element_value(
    element: &IUIAutomationElement,
    live: &UiElementSnapshot,
    window: &WindowSnapshot,
    value: &str,
    commit_tracker: &ControlCommitTracker,
) -> Result<&'static str, ControlFailure> {
    let current_is_password =
        before_control_commit(unsafe { element.CurrentIsPassword() }.map_err(|error| {
            platform_error(
                "verify UI Automation password state before setting value",
                error,
            )
        }))?;
    if live.password || current_is_password.as_bool() {
        return Err(ControlFailure::pre_commit_as(
            PreCommitFailureKind::Refused,
            invalid(
            "Refusing set_value on a password element. Secrets must never enter computer-use tool arguments.",
            ),
        ));
    }
    let pattern = before_control_commit(unsafe {
        element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
    }
    .map_err(|_| {
        invalid(
            "Target does not expose the UI Automation Value pattern. Use an approved foreground input only if the user explicitly requested it.",
        )
    }))?;
    let read_only = before_control_commit(
        unsafe { pattern.CurrentIsReadOnly() }
            .map_err(|error| platform_error("read UI Automation value state", error)),
    )?;
    if read_only.as_bool() {
        return Err(ControlFailure::pre_commit_as(
            PreCommitFailureKind::Refused,
            invalid("Target UI Automation value is read-only."),
        ));
    }
    // The Windows EDIT accessibility proxy may focus its HWND in SetValue.
    // For an identity-checked native EDIT child, WM_SETTEXT performs the
    // same value replacement without activating the desktop window.
    let native_edit = native_child_handle(element, window, "edit");
    commit_tracker.result(commit_tracker.mark())?;
    let route = if let Some(handle) = native_edit {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            SendMessageTimeoutW, SMTO_ABORTIFHUNG, SMTO_BLOCK, WM_SETTEXT,
        };
        let text: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        let mut retained = 0_usize;
        let delivered = unsafe {
            SendMessageTimeoutW(
                handle,
                WM_SETTEXT,
                WPARAM(0),
                LPARAM(text.as_ptr() as isize),
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                1_000,
                Some(&mut retained),
            )
        };
        commit_tracker.result(if delivered.0 != 0 && retained != 0 {
            Ok(())
        } else {
            Err(invalid(
                "Native edit did not acknowledge the value replacement. Observe before retrying.",
            ))
        })?;
        "native_edit_value"
    } else {
        commit_tracker.result(
            unsafe { pattern.SetValue(&BSTR::from(value)) }
                .map_err(|error| platform_error("set UI Automation value", error)),
        )?;
        "value_pattern"
    };
    let actual = commit_tracker.result(
        unsafe { pattern.CurrentValue() }
            .map_err(|error| platform_error("verify UI Automation value", error)),
    )?;
    commit_tracker.result(if actual == value { Ok(()) } else {
        Err(invalid("The application did not retain the requested value exactly. Input was delivered; inspect the window before retrying."))
    })?;
    Ok(route)
}

fn native_checkbox_message(
    target: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> Result<usize, CoreError> {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, SMTO_ABORTIFHUNG, SMTO_BLOCK,
    };
    let mut result = 0;
    let delivered = unsafe {
        SendMessageTimeoutW(
            target,
            message,
            WPARAM(wparam),
            LPARAM(lparam),
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            1_000,
            Some(&mut result),
        )
    };
    if delivered.0 == 0 {
        return Err(invalid(
            "Native checkbox message was not acknowledged. Observe before retrying.",
        ));
    }
    Ok(result)
}

fn native_checkbox_toggle(
    button: HWND,
    window: &WindowSnapshot,
    observed_state: i32,
    commit_tracker: &ControlCommitTracker,
) -> Result<(), ControlFailure> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetDlgCtrlID, GetParent, GetWindowLongW, IsChild, BM_GETCHECK, BM_SETCHECK, BN_CLICKED,
        BS_3STATE, BS_AUTO3STATE, BS_AUTOCHECKBOX, BS_CHECKBOX, BS_TYPEMASK, GWL_STYLE, WM_COMMAND,
    };
    let style = unsafe { GetWindowLongW(button, GWL_STYLE) } & BS_TYPEMASK;
    let automatic = style == BS_AUTOCHECKBOX || style == BS_AUTO3STATE;
    let three_state = style == BS_3STATE || style == BS_AUTO3STATE;
    if !automatic && style != BS_CHECKBOX && style != BS_3STATE {
        return Err(ControlFailure::pre_commit_as(
            PreCommitFailureKind::Refused,
            invalid("Native BUTTON has no supported checkbox style; no input was sent."),
        ));
    }
    let parent = before_control_commit(
        unsafe { GetParent(button) }
            .map_err(|error| platform_error("resolve native checkbox parent", error)),
    )?;
    let mut parent_pid = 0;
    unsafe { GetWindowThreadProcessId(parent, Some(&mut parent_pid)) };
    let control_id = unsafe { GetDlgCtrlID(button) };
    if parent_pid != window.pid
        || (parent != hwnd(window.id) && !unsafe { IsChild(hwnd(window.id), parent) }.as_bool())
        || !(0..=u16::MAX as i32).contains(&control_id)
        || !unsafe { IsWindowEnabled(button) }.as_bool()
        || !unsafe { IsWindowEnabled(parent) }.as_bool()
    {
        return Err(ControlFailure::pre_commit_as(
            PreCommitFailureKind::ObservationStale,
            invalid("Native checkbox parent, identifier, or enabled state changed. Capture again."),
        ));
    }
    let state = before_control_commit(native_checkbox_message(button, BM_GETCHECK, 0, 0))?;
    if state > if three_state { 2 } else { 1 } || state as i32 != observed_state {
        return Err(ControlFailure::pre_commit_as(
            PreCommitFailureKind::ObservationStale,
            invalid("Native checkbox state no longer matches the observed Toggle state."),
        ));
    }
    // BM_CLICK sends mouse-down/up, which can activate a background HWND.
    // For exact standard checkbox styles, reproduce the documented state
    // transition and the application's actual BN_CLICKED notification.
    // Manual checkbox styles leave the transition to that app handler.
    commit_tracker.result(commit_tracker.mark())?;
    if automatic {
        let next = (state + 1) % if three_state { 3 } else { 2 };
        commit_tracker.result(native_checkbox_message(button, BM_SETCHECK, next, 0))?;
        let retained = commit_tracker.result(native_checkbox_message(button, BM_GETCHECK, 0, 0))?;
        if retained != next {
            return Err(commit_tracker.failure(invalid(
                "Native checkbox did not retain its state transition. Observe before retrying.",
            )));
        }
    }
    // The exact parent receives control ID + HWND, just as for a real
    // checkbox click. A state-only BM_SETCHECK is never a successful action.
    commit_tracker.result(native_checkbox_message(
        parent,
        WM_COMMAND,
        control_id as usize | ((BN_CLICKED as usize) << 16),
        button.0 as isize,
    ))?;
    Ok(())
}

fn ensure_focused_target_is_not_password(window: &WindowSnapshot) -> Result<(), CoreError> {
    let _apartment = ComApartment::initialize().map_err(|error| {
        invalid(format!(
            "Cannot verify the focused element's password state: {error}"
        ))
    })?;
    let automation = create_automation().map_err(|error| {
        invalid(format!(
            "Cannot verify the focused element's password state: {error}"
        ))
    })?;
    let focused = unsafe { automation.GetFocusedElement() }.map_err(|error| {
        invalid(format!(
            "Cannot verify the focused UI Automation element: {error}"
        ))
    })?;
    let pid = unsafe { focused.CurrentProcessId() }
        .map_err(|error| invalid(format!("Cannot verify focused element owner: {error}")))?;
    if pid < 0 || pid as u32 != window.pid {
        let bounds = unsafe { focused.CurrentBoundingRectangle() }.map_err(|error| {
            invalid(format!(
                "Cannot prove that the cross-process focused element belongs to the approved window: {error}"
            ))
        })?;
        let point = (
            bounds.left.saturating_add((bounds.right - bounds.left) / 2),
            bounds.top.saturating_add((bounds.bottom - bounds.top) / 2),
        );
        ensure_point_targets_window(point, window).map_err(|_| {
            invalid(
                "UI Automation reports that keyboard focus left the approved target window; no text was sent.",
            )
        })?;
    }
    let password = unsafe { focused.CurrentIsPassword() }.map_err(|error| {
        invalid(format!(
            "Cannot verify the focused element's password state: {error}"
        ))
    })?;
    if password.as_bool() {
        return Err(invalid(
            "The focused UI Automation element is a password field. Secrets must never enter computer-use tool arguments.",
        ));
    }
    Ok(())
}

fn hwnd(window_id: u64) -> HWND {
    HWND(window_id as usize as *mut c_void)
}

fn focus_window(
    window: &WindowSnapshot,
    commit_tracker: &ControlCommitTracker,
) -> Result<(), CoreError> {
    let handle = hwnd(window.id);
    unsafe {
        if !IsWindow(Some(handle)).as_bool() {
            return Err(invalid(format!("Window {} is no longer valid.", window.id)));
        }
        if IsIconic(handle).as_bool() {
            commit_tracker.mark()?;
            let _ = ShowWindow(handle, SW_RESTORE);
            thread::sleep(INPUT_SETTLE);
        }
        if GetForegroundWindow() != handle {
            commit_tracker.mark()?;
            if !SetForegroundWindow(handle).as_bool() {
                return Err(invalid(format!(
                    "Windows refused to focus window {}. Bring it to the foreground and retry.",
                    window.id
                )));
            }
        }
    }
    thread::sleep(INPUT_SETTLE);
    if unsafe { GetForegroundWindow() } != handle {
        return Err(invalid(format!(
            "Window {} did not become foreground; no input was sent.",
            window.id
        )));
    }
    let (_, verified) = current_window(window)?;
    if verified.pid != window.pid || unsafe { GetForegroundWindow() } != handle {
        return Err(invalid(
            "Target identity or foreground focus changed before input; no input was sent.",
        ));
    }
    Ok(())
}

fn ensure_target_foreground(window: &WindowSnapshot) -> Result<(), CoreError> {
    let (_, verified) = current_window(window)?;
    if verified.pid != window.pid || unsafe { GetForegroundWindow() } != hwnd(window.id) {
        return Err(invalid(
            "Foreground focus changed before input; refusing to send input to an uncertain target.",
        ));
    }
    Ok(())
}

fn ensure_observation_fresh_after_focus(
    observed: &ObservedWindow,
    window: &WindowSnapshot,
) -> Result<(), CoreError> {
    let expected_signature = observed
        .screenshot_signature
        .as_deref()
        .ok_or_else(|| invalid("Foreground input requires a captured-window observation."))?;
    let capture = capture_window(window, CaptureOptions::pixels_only())?;
    if capture.snapshot.title != observed.snapshot.title
        || capture.snapshot.x != observed.snapshot.x
        || capture.snapshot.y != observed.snapshot.y
        || capture.snapshot.width != observed.snapshot.width
        || capture.snapshot.height != observed.snapshot.height
    {
        return Err(invalid(
            "Target geometry or title changed while acquiring focus. Re-observe before input.",
        ));
    }
    let current_signature = screenshot_signature(&capture.png)
        .ok_or_else(|| invalid("Could not verify the focused window screenshot."))?;
    if !screenshot_signatures_match(expected_signature, &current_signature) {
        return Err(invalid(
            "Target content changed while acquiring focus. No input was sent; re-observe the window.",
        ));
    }
    ensure_target_foreground(window)
}

fn ensure_target_patch_fresh(
    observed: &ObservedWindow,
    window: &WindowSnapshot,
    point: (i32, i32),
) -> Result<(), CoreError> {
    let expected = observed.screenshot_guard.as_ref().ok_or_else(|| {
        invalid("Target-local freshness data is unavailable. Capture the window again.")
    })?;
    let capture = capture_window(window, CaptureOptions::pixels_only())?;
    let current = screenshot_guard(&capture.png)
        .ok_or_else(|| invalid("Could not build target-local freshness evidence."))?;
    let normalized_x = if window.width <= 1 {
        0.0
    } else {
        (point.0 - window.x) as f64 / window.width.saturating_sub(1) as f64
    };
    let normalized_y = if window.height <= 1 {
        0.0
    } else {
        (point.1 - window.y) as f64 / window.height.saturating_sub(1) as f64
    };
    if !screenshot_guard_patch_matches(expected, &current, normalized_x, normalized_y) {
        return Err(invalid(
            "The approved target region changed since observation. No pointer input was sent; capture again.",
        ));
    }
    Ok(())
}

fn move_cursor_frame(point: (i32, i32), context: &str) -> Result<(), CoreError> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) }.max(1);
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) }.max(1);
    let absolute = |position: i32, origin: i32, size: i32| -> i32 {
        (((i64::from(position) - i64::from(origin)) * 65536 + 32768) / i64::from(size))
            .clamp(0, 65535) as i32
    };
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: absolute(point.0, left, width),
                dy: absolute(point.1, top, height),
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                ..Default::default()
            },
        },
    };
    if unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) } != 1 {
        return Err(invalid(format!(
            "Windows refused pointer motion during {context}"
        )));
    }
    let actual = cursor_position()?;
    if actual.0.abs_diff(point.0) > 1 || actual.1.abs_diff(point.1) > 1 {
        return Err(invalid(format!(
            "Windows moved the cursor to ({}, {}) instead of ({}, {}); refusing uncertain multi-display input.",
            actual.0, actual.1, point.0, point.1
        )));
    }
    super::super::computer_pointer_feedback::show_at(point);
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PointerApproach {
    Foreground,
    FeedbackOnly,
}

/// One motion path for native input and semantic feedback. Frames carry no
/// button events; background delivery never moves the physical cursor.
fn approach_pointer(
    point: (i32, i32),
    window: &WindowSnapshot,
    tracker: &ControlCommitTracker,
    context: &str,
    delivery: PointerApproach,
) -> Result<(), CoreError> {
    use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONULL};
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    tracker.progress(crate::observed_interaction::InteractionPhase::Moving);
    let from = cursor_position()?;
    let scale = f64::from(unsafe { GetDpiForWindow(hwnd(window.id)) }.max(96)) / 96.0;
    let (duration, points) = if super::super::computer_pointer_feedback::reduced_motion() {
        (Duration::ZERO, vec![point])
    } else {
        super::pointer_motion_path(from, point, scale)
    };
    let started = Instant::now();
    let mut previous = from;
    super::super::computer_pointer_feedback::show_at(from);
    for (index, next) in points.iter().copied().enumerate() {
        let due = duration.mul_f64((index + 1) as f64 / points.len() as f64);
        thread::sleep(due.saturating_sub(started.elapsed()));
        tracker.ensure_running()?;
        if delivery == PointerApproach::FeedbackOnly {
            super::super::computer_pointer_feedback::show_at(next);
            continue;
        }
        ensure_target_foreground(window)?;
        ensure_cursor_at(previous, context)?;
        for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            ensure_mouse_button_not_physically_pressed(button)?;
        }
        // A straight path can cross empty space between offset monitors.
        // Do not ask Windows to clamp those intermediate physical points.
        let screen = unsafe {
            MonitorFromPoint(
                windows::Win32::Foundation::POINT {
                    x: next.0,
                    y: next.1,
                },
                MONITOR_DEFAULTTONULL,
            )
        };
        if screen.0.is_null() && index + 1 < points.len() {
            continue;
        }
        move_cursor_frame(next, context)?;
        previous = next;
    }
    tracker.ensure_running()
}

fn ensure_cursor_at(point: (i32, i32), context: &str) -> Result<(), CoreError> {
    let actual = cursor_position()?;
    if actual.0.abs_diff(point.0) > 2 || actual.1.abs_diff(point.1) > 2 {
        return Err(invalid(format!(
            "Cursor moved during {context}, indicating user takeover; no further input was sent."
        )));
    }
    Ok(())
}

fn ensure_point_targets_window(
    point: (i32, i32),
    window: &WindowSnapshot,
) -> Result<(), CoreError> {
    super::super::computer_pointer_feedback::avoid_status_at(point);
    let hit = unsafe {
        WindowFromPoint(windows::Win32::Foundation::POINT {
            x: point.0,
            y: point.1,
        })
    };
    if hit.0.is_null() {
        return Err(invalid(
            "No live window owns the approved input point; refusing pointer input.",
        ));
    }
    let root = unsafe { GetAncestor(hit, GA_ROOT) };
    let root = if root.0.is_null() { hit } else { root };
    let mut owner_pid = 0_u32;
    let thread_id = unsafe { GetWindowThreadProcessId(root, Some(&mut owner_pid)) };
    if thread_id == 0 || root != hwnd(window.id) || owner_pid != window.pid {
        return Err(invalid(
            "Another window or overlay owns the approved input point; no pointer input was sent.",
        ));
    }
    Ok(())
}

fn image_point(
    x: Option<f64>,
    y: Option<f64>,
    coordinate_space: CoordinateSpace,
    observed: &ObservedWindow,
    current: &WindowSnapshot,
    label: &str,
) -> Result<(i32, i32), CoreError> {
    let x = x.ok_or_else(|| invalid(format!("{label} requires x.")))?;
    let y = y.ok_or_else(|| invalid(format!("{label} requires y.")))?;
    let (image_width, image_height) = observed
        .image_width
        .zip(observed.image_height)
        .ok_or_else(|| {
            invalid(format!(
                "{label} needs captured-image coordinates. Run computer_observe capture_window first."
            ))
        })?;
    let (native_image_width, native_image_height) = observed
        .native_image_width
        .zip(observed.native_image_height)
        .ok_or_else(|| {
            invalid(format!(
                "{label} needs capture transform metadata. Capture the window again."
            ))
        })?;
    if native_image_width.abs_diff(current.width) > 2
        || native_image_height.abs_diff(current.height) > 2
    {
        return Err(invalid(
            "The Windows capture frame and screen window geometry do not align safely for raw coordinates. Use a semantic element_id or re-observe after moving the window.",
        ));
    }
    let (image_x, image_y) = match coordinate_space {
        CoordinateSpace::CapturedImagePixels => {
            if !x.is_finite()
                || !y.is_finite()
                || x < 0.0
                || y < 0.0
                || x >= image_width as f64
                || y >= image_height as f64
            {
                return Err(invalid(format!(
                    "Point ({x}, {y}) is outside the captured image bounds {image_width}x{image_height}."
                )));
            }
            (x, y)
        }
        CoordinateSpace::Normalized => {
            if !x.is_finite()
                || !y.is_finite()
                || !(0.0..=1.0).contains(&x)
                || !(0.0..=1.0).contains(&y)
            {
                return Err(invalid(format!(
                    "Normalized point ({x}, {y}) must be inside 0..1."
                )));
            }
            (
                x * image_width.saturating_sub(1) as f64,
                y * image_height.saturating_sub(1) as f64,
            )
        }
    };
    let native_x = if image_width <= 1 || native_image_width <= 1 {
        0.0
    } else {
        image_x * native_image_width.saturating_sub(1) as f64 / image_width.saturating_sub(1) as f64
    };
    let native_y = if image_height <= 1 || native_image_height <= 1 {
        0.0
    } else {
        image_y * native_image_height.saturating_sub(1) as f64
            / image_height.saturating_sub(1) as f64
    };
    let local_x = native_x
        .round()
        .clamp(0.0, current.width.saturating_sub(1) as f64) as i32;
    let local_y = native_y
        .round()
        .clamp(0.0, current.height.saturating_sub(1) as f64) as i32;
    Ok((current.x + local_x, current.y + local_y))
}

fn resolve_pointer_target(
    element_id: Option<&str>,
    x: Option<f64>,
    y: Option<f64>,
    coordinate_space: CoordinateSpace,
    observed: &ObservedWindow,
    current: &WindowSnapshot,
    label: &str,
) -> Result<ResolvedPointerTarget, CoreError> {
    if let Some(element_id) = element_id {
        let element = super::semantic_element(observed, element_id, label)?;
        if !element.enabled {
            return Err(invalid(format!(
                "Element {element_id} is disabled; refusing {label}."
            )));
        }
        let live = resolve_live_element(current, observed, element)?;
        let point = live_element_point(&live, current)?;
        return Ok(ResolvedPointerTarget {
            point,
            live: Some(live),
        });
    }
    Ok(ResolvedPointerTarget {
        point: image_point(x, y, coordinate_space, observed, current, label)?,
        live: None,
    })
}

#[derive(Clone, Copy)]
enum MouseButton {
    Left,
    Right,
    Middle,
}

fn mouse_button(value: Option<&str>) -> Result<MouseButton, CoreError> {
    match value.unwrap_or("left").trim().to_ascii_lowercase().as_str() {
        "left" => Ok(MouseButton::Left),
        "right" => Ok(MouseButton::Right),
        "middle" => Ok(MouseButton::Middle),
        other => Err(invalid(format!("Unsupported mouse button '{other}'."))),
    }
}

fn mouse_button_flags(button: MouseButton) -> (MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS) {
    match button {
        MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
    }
}

fn mouse_input(flags: MOUSE_EVENT_FLAGS, data: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0x4E58_4132,
            },
        },
    }
}

fn ensure_mouse_button_not_physically_pressed(button: MouseButton) -> Result<(), CoreError> {
    let key = match button {
        MouseButton::Left => VK_LBUTTON,
        MouseButton::Right => VK_RBUTTON,
        MouseButton::Middle => VK_MBUTTON,
    };
    if unsafe { GetAsyncKeyState(key.0 as i32) } < 0 {
        return Err(invalid(
            "The requested mouse button is physically pressed, indicating user takeover; no mouse button input was sent.",
        ));
    }
    Ok(())
}

fn send_mouse_click(button: MouseButton) -> Result<(), CoreError> {
    let (down, up) = mouse_button_flags(button);
    let inputs = [mouse_input(down, 0), mouse_input(up, 0)];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) } as usize;
    if sent != inputs.len() {
        let emergency = [mouse_input(up, 0)];
        let released = unsafe { SendInput(&emergency, std::mem::size_of::<INPUT>() as i32) };
        return Err(CoreError::Internal(format!(
            "Windows accepted {sent} of {} mouse-click events; emergency release accepted {released}. Mouse effect is uncertain.",
            inputs.len()
        )));
    }
    Ok(())
}

fn send_mouse_button(button: MouseButton, down: bool) -> Result<(), CoreError> {
    let (down_flag, up_flag) = mouse_button_flags(button);
    let inputs = [mouse_input(if down { down_flag } else { up_flag }, 0)];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != 1 {
        return Err(CoreError::Internal(format!(
            "Windows rejected an injected mouse {} event; effect is uncertain.",
            if down { "press" } else { "release" }
        )));
    }
    Ok(())
}

fn send_scroll_steps(horizontal: bool, steps: i32) -> Result<(), CoreError> {
    let delta = if horizontal {
        steps.saturating_mul(WHEEL_DELTA as i32)
    } else {
        steps.saturating_mul(-(WHEEL_DELTA as i32))
    };
    let inputs = [mouse_input(
        if horizontal {
            MOUSEEVENTF_HWHEEL
        } else {
            MOUSEEVENTF_WHEEL
        },
        delta as u32,
    )];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != 1 {
        return Err(CoreError::Internal(
            "Windows rejected an injected scroll event; effect is uncertain.".to_string(),
        ));
    }
    Ok(())
}

fn named_key(value: &str) -> Result<VIRTUAL_KEY, CoreError> {
    let normalized = value.trim().to_ascii_lowercase();
    let key = match normalized.as_str() {
        "ctrl" | "control" => VK_CONTROL,
        "alt" | "option" => VK_MENU,
        "shift" => VK_SHIFT,
        "meta" | "win" | "windows" | "command" | "cmd" => VK_LWIN,
        "enter" | "return" => VK_RETURN,
        "tab" => VK_TAB,
        "space" => VK_SPACE,
        "backspace" => VK_BACK,
        "delete" | "del" => VK_DELETE,
        "escape" | "esc" => VK_ESCAPE,
        "up" | "arrowup" => VK_UP,
        "down" | "arrowdown" => VK_DOWN,
        "left" | "arrowleft" => VK_LEFT,
        "right" | "arrowright" => VK_RIGHT,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" => VK_PRIOR,
        "pagedown" => VK_NEXT,
        value if value.starts_with('f') && value[1..].parse::<u16>().is_ok() => {
            let number = value[1..].parse::<u16>().unwrap_or(0);
            if !(1..=12).contains(&number) {
                return Err(invalid(format!("Unsupported key name '{value}'.")));
            }
            VIRTUAL_KEY(VK_F1.0 + number - 1)
        }
        "plus" => {
            let mapped = unsafe { VkKeyScanW('+' as u16) };
            if mapped == -1 {
                return Err(invalid("Current keyboard layout cannot represent plus."));
            }
            VIRTUAL_KEY(mapped as u16 & 0x00ff)
        }
        _ => {
            let mut chars = value.chars();
            let Some(character) = chars.next() else {
                return Err(invalid("Key sequence contains an empty key."));
            };
            if chars.next().is_some() {
                return Err(invalid(format!("Unsupported key name '{value}'.")));
            }
            let mapped = unsafe { VkKeyScanW(character as u16) };
            if mapped == -1 {
                return Err(invalid(format!(
                    "Current keyboard layout cannot represent key '{value}'."
                )));
            }
            VIRTUAL_KEY(mapped as u16 & 0x00ff)
        }
    };
    Ok(key)
}

fn virtual_key_input(key: VIRTUAL_KEY, key_up: bool) -> INPUT {
    let mut flags = if matches!(
        key,
        value if value == VK_DELETE
            || value == VK_DOWN
            || value == VK_END
            || value == VK_HOME
            || value == VK_LEFT
            || value == VK_LWIN
            || value == VK_NEXT
            || value == VK_PRIOR
            || value == VK_RIGHT
            || value == VK_UP
    ) {
        KEYEVENTF_EXTENDEDKEY
    } else {
        Default::default()
    };
    if key_up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0x4E58_4132,
            },
        },
    }
}

fn send_key_sequence(sequence: &str) -> Result<(), CoreError> {
    let parts: Vec<&str> = sequence
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() || parts.len() > 8 {
        return Err(invalid(
            "key_sequence must contain between one and eight '+'-separated keys.",
        ));
    }
    let keys: Vec<VIRTUAL_KEY> = parts
        .iter()
        .map(|part| named_key(part))
        .collect::<Result<_, _>>()?;
    if keys
        .iter()
        .any(|key| unsafe { GetAsyncKeyState(key.0 as i32) } < 0)
    {
        return Err(invalid(
            "A requested shortcut key is already physically pressed, indicating user takeover; no key input was sent.",
        ));
    }
    let mut inputs = Vec::with_capacity(keys.len().saturating_mul(2));
    for key in &keys {
        inputs.push(virtual_key_input(*key, false));
    }
    for key in keys.iter().rev() {
        inputs.push(virtual_key_input(*key, true));
    }
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) } as usize;
    if sent != inputs.len() {
        let emergency_releases = keys
            .iter()
            .rev()
            .map(|key| virtual_key_input(*key, true))
            .collect::<Vec<_>>();
        let released =
            unsafe { SendInput(&emergency_releases, std::mem::size_of::<INPUT>() as i32) } as usize;
        return Err(CoreError::Internal(format!(
            "Windows accepted {sent} of {} shortcut events; emergency release accepted {released} of {}. Action effect is uncertain.",
            inputs.len(),
            emergency_releases.len()
        )));
    }
    Ok(())
}

enum TextInputOperation {
    Text(String),
    Key(VIRTUAL_KEY),
}

fn text_input_operations(text: &str) -> Vec<TextInputOperation> {
    let mut operations = Vec::new();
    let mut run = String::new();
    let mut characters = text.chars().peekable();
    let flush = |run: &mut String, operations: &mut Vec<TextInputOperation>| {
        if !run.is_empty() {
            operations.push(TextInputOperation::Text(std::mem::take(run)));
        }
    };
    while let Some(character) = characters.next() {
        match character {
            '\r' => {
                flush(&mut run, &mut operations);
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                operations.push(TextInputOperation::Key(VK_RETURN));
            }
            '\n' => {
                flush(&mut run, &mut operations);
                operations.push(TextInputOperation::Key(VK_RETURN));
            }
            '\t' => {
                flush(&mut run, &mut operations);
                operations.push(TextInputOperation::Key(VK_TAB));
            }
            character => {
                run.push(character);
            }
        }
    }
    flush(&mut run, &mut operations);
    operations
}

fn append_unicode_text_inputs(text: &str, inputs: &mut Vec<INPUT>) {
    const NEXA_INPUT_MARKER: usize = 0x4E58_4132;
    for unit in text.encode_utf16() {
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: NEXA_INPUT_MARKER,
                },
            },
        });
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: NEXA_INPUT_MARKER,
                },
            },
        });
    }
}

fn type_text_checked(
    text: &str,
    window: &WindowSnapshot,
    target: Option<&LiveElement>,
) -> Result<(), CoreError> {
    let operations = text_input_operations(text);
    ensure_target_foreground(window)?;
    if let Some(target) = target {
        let focused = unsafe { target.element.CurrentHasKeyboardFocus() }
            .map_err(|error| {
                invalid(format!(
                    "Cannot revalidate the semantic text target focus: {error}"
                ))
            })?
            .as_bool();
        let password = unsafe { target.element.CurrentIsPassword() }
            .map_err(|error| {
                invalid(format!(
                    "Cannot revalidate the semantic text target password state: {error}"
                ))
            })?
            .as_bool();
        if !focused {
            return Err(invalid(
                "The exact semantic text target lost keyboard focus.",
            ));
        }
        if password {
            return Err(invalid("The semantic text target became a password field."));
        }
    } else {
        ensure_focused_target_is_not_password(window)?;
    }

    let mut inputs = Vec::with_capacity(text.encode_utf16().count().saturating_mul(2));
    let mut control_keys = Vec::new();
    for operation in &operations {
        match operation {
            TextInputOperation::Text(chunk) => append_unicode_text_inputs(chunk, &mut inputs),
            TextInputOperation::Key(key) => {
                if unsafe { GetAsyncKeyState(key.0 as i32) } < 0 {
                    return Err(invalid(
                        "A text-control key is physically pressed, indicating user takeover; no text input was sent.",
                    ));
                }
                control_keys.push(*key);
                inputs.push(virtual_key_input(*key, false));
                inputs.push(virtual_key_input(*key, true));
            }
        }
    }
    if inputs.is_empty() {
        return Ok(());
    }
    // Submit the complete approved text/key sequence in one SendInput
    // batch. There is no inter-chunk window in which human keystrokes can
    // be silently mixed into a still-running Agent action.
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) } as usize;
    if sent != inputs.len() {
        let emergency_releases = control_keys
            .iter()
            .rev()
            .map(|key| virtual_key_input(*key, true))
            .collect::<Vec<_>>();
        let released = if emergency_releases.is_empty() {
            0
        } else {
            unsafe { SendInput(&emergency_releases, std::mem::size_of::<INPUT>() as i32) }
        };
        return Err(CoreError::Internal(format!(
            "Windows accepted {sent} of {} text input events; emergency release accepted {released}. Action effect is uncertain.",
            inputs.len()
        )));
    }
    Ok(())
}

fn drag_mouse(
    from: (i32, i32),
    to: (i32, i32),
    window: &WindowSnapshot,
    from_live: Option<&LiveElement>,
    to_live: Option<&LiveElement>,
    duration_ms: u64,
    tracker: &ControlCommitTracker,
) -> Result<(), CoreError> {
    tracker.ensure_running()?;
    ensure_target_foreground(window)?;
    ensure_cursor_at(from, "drag preparation")?;
    ensure_point_targets_window(from, window)?;
    if let Some(live) = from_live {
        ensure_point_matches_live_element(live, from)?;
    }
    ensure_mouse_button_not_physically_pressed(MouseButton::Left)?;
    send_mouse_button(MouseButton::Left, true)?;
    let mut movement_error = None;
    let mut previous = from;
    let frames = duration_ms.div_ceil(16).clamp(7, 188);
    for frame in 1..=frames {
        if let Err(error) = tracker.ensure_running() {
            movement_error = Some(error);
            break;
        }
        if unsafe { GetForegroundWindow() } != hwnd(window.id) {
            movement_error = Some(CoreError::Internal(
                "Foreground focus changed during drag; released the mouse button and stopped. Action effect is uncertain."
                    .to_string(),
            ));
            break;
        }
        if let Err(error) = ensure_cursor_at(previous, "drag") {
            movement_error = Some(CoreError::Internal(format!(
                "User takeover occurred after drag button press; action effect is uncertain: {error}"
            )));
            break;
        }
        let progress = frame as f64 / frames as f64;
        let eased = 1.0 - (1.0 - progress).powi(3);
        let x = from.0 as f64 + (to.0 - from.0) as f64 * eased;
        let y = from.1 as f64 + (to.1 - from.1) as f64 * eased;
        let next = (x.round() as i32, y.round() as i32);
        if let Err(error) = ensure_point_targets_window(next, window) {
            movement_error = Some(CoreError::Internal(format!(
                "Another surface owns the next drag point; released before moving there. Action effect is uncertain: {error}"
            )));
            break;
        }
        if let Err(error) = move_cursor_frame(next, "drag mouse") {
            movement_error = Some(CoreError::Internal(format!(
                "Drag cursor movement failed after button press; action effect is uncertain: {error}"
            )));
            break;
        }
        if let Err(error) = ensure_point_targets_window(next, window) {
            movement_error = Some(CoreError::Internal(format!(
                "Another surface intercepted the drag path after button press; action effect is uncertain: {error}"
            )));
            break;
        }
        previous = next;
        thread::sleep(Duration::from_millis((duration_ms / frames).max(1)));
    }
    if movement_error.is_none() {
        if let Some(live) = to_live {
            if let Err(error) = ensure_point_matches_live_element(live, to) {
                movement_error = Some(CoreError::Internal(format!(
                    "The semantic drag destination changed after button press; action effect is uncertain: {error}"
                )));
            }
        }
    }
    let mut release = send_mouse_button(MouseButton::Left, false);
    if release.is_err() {
        thread::sleep(Duration::from_millis(5));
        release = send_mouse_button(MouseButton::Left, false).map_err(|error| {
            CoreError::Internal(format!(
                "Failed twice to release the injected drag button. The mouse button may remain pressed; action effect is uncertain: {error}"
            ))
        });
    }
    if let Some(error) = movement_error {
        if let Err(release_error) = release {
            return Err(CoreError::Internal(format!(
                "{error} Emergency input cleanup also failed: {release_error}"
            )));
        }
        return Err(error);
    }
    release
}

pub(super) fn cursor_position() -> Result<(i32, i32), CoreError> {
    let _physical_pixels = PhysicalPixels::enter()?;
    let mut point = windows::Win32::Foundation::POINT::default();
    unsafe { GetCursorPos(&mut point) }
        .map_err(|error| platform_error("read cursor position", error))?;
    Ok((point.x, point.y))
}

pub(super) fn control_window(
    action: ControlAction,
    args: &ControlArgs,
    observed: &ObservedWindow,
    final_capture_options: CaptureOptions,
    commit_tracker: &ControlCommitTracker,
) -> Result<ControlOutcome, ControlFailure> {
    let physical_epoch = super::super::computer_input::physical_epoch();
    let window_epoch = super::super::computer_input::window_epoch(observed.snapshot.id);
    if window_epoch != observed.snapshot.input_epoch {
        return Err(ControlFailure::pre_commit_as(
            PreCommitFailureKind::UserTakeover,
            invalid("The user interacted after observation."),
        ));
    }
    let _physical_pixels = before_control_commit_as(
        PreCommitFailureKind::CoordinateTransform,
        PhysicalPixels::enter(),
    )?;
    let (_, mut current) = before_control_commit_as(
        PreCommitFailureKind::TargetUnavailable,
        current_window(&observed.snapshot),
    )?;
    if current.title != observed.snapshot.title {
        return Err(ControlFailure::pre_commit_as(
            PreCommitFailureKind::ObservationStale,
            invalid(
            "Desktop observation is stale because the target window title changed. Observe it again before acting.",
            ),
        ));
    }

    let requires_captured_state = action != ControlAction::FocusWindow;
    if requires_captured_state && observed.screenshot_signature.is_none() {
        return Err(ControlFailure::pre_commit_as(
            PreCommitFailureKind::ObservationStale,
            invalid(
            "This action requires capture_window first. list_windows observations can only be used with focus_window.",
            ),
        ));
    }
    if requires_captured_state
        && (current.width != observed.snapshot.width
            || current.height != observed.snapshot.height
            || current.x != observed.snapshot.x
            || current.y != observed.snapshot.y)
    {
        return Err(ControlFailure::pre_commit_as(
            PreCommitFailureKind::ObservationStale,
            invalid(
            "Desktop observation is stale because the target window moved or resized. Observe it again before acting.",
            ),
        ));
    }

    let pre_action_capture = if current.minimized {
        if requires_captured_state {
            return Err(ControlFailure::pre_commit_as(
                PreCommitFailureKind::ObservationStale,
                invalid(
                "Target window is minimized. Use focus_window with a fresh list_windows observation to restore it, then capture it before acting.",
                ),
            ));
        }
        None
    } else {
        Some(before_control_commit_as(
            PreCommitFailureKind::CaptureFailed,
            capture_window(&current, CaptureOptions::pixels_only()),
        )?)
    };
    if let (Some(expected), Some(capture)) = (
        observed.screenshot_signature.as_ref(),
        pre_action_capture.as_ref(),
    ) {
        let screenshot_changed = screenshot_signature(&capture.png)
            .is_none_or(|signature| !screenshot_signatures_match(expected, &signature));
        if screenshot_changed {
            return Err(ControlFailure::pre_commit_as(
                PreCommitFailureKind::ObservationStale,
                invalid(
                "Desktop observation is stale because the target window changed materially. Observe it again before acting.",
                ),
            ));
        }
    }

    let pre_action_hash = pre_action_capture
        .as_ref()
        .map(|capture| blake3::hash(&capture.png).to_hex().to_string())
        .unwrap_or_else(|| "unavailable_before_restore".to_string());
    let pre_action_signature = pre_action_capture
        .as_ref()
        .and_then(|capture| screenshot_signature(&capture.png));
    let coordinate_space =
        before_control_commit(CoordinateSpace::parse(args.coordinate_space.as_deref()))?;
    let mut route = "global_input";
    let mut delivery = "foreground";

    // A semantic single click can run without activating the window. Probe
    // the actual pattern first; only auto mode may fall back before input.
    let mut effective_action = action;
    if action == ControlAction::Click && permits_semantic_click(args) {
        let element_id = args.element_id.as_deref().expect("semantic click target");
        let expected =
            before_control_commit(super::semantic_element(observed, element_id, "click"))?;
        let live = before_control_commit_as(
            PreCommitFailureKind::ObservationStale,
            resolve_live_element(&current, observed, expected),
        )?;
        if supports_semantic_invoke(&live.element) {
            effective_action = ControlAction::Invoke;
        }
    }
    if args.delivery == Some(ControlDelivery::Background)
        && effective_action == ControlAction::Click
    {
        return Err(ControlFailure::pre_commit_as(PreCommitFailureKind::Refused,
            invalid("The target has no usable background invocation pattern. No foreground input was sent.")));
    }

    if observed.screenshot_signature.is_some()
        && !modal_owner_matches(
            observed.snapshot.modal_owner.as_deref(),
            observed_modal_owner(&current).as_deref(),
        )
    {
        return Err(ControlFailure::pre_commit_as(
            PreCommitFailureKind::ObservationStale,
            invalid("The observed modal owner relationship changed before input. Capture again."),
        ));
    }

    let foreground = !matches!(
        effective_action,
        ControlAction::Invoke | ControlAction::SetValue
    );
    let epoch = if foreground {
        physical_epoch
    } else {
        window_epoch
    };
    *commit_tracker
        .input_watch
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some((observed.snapshot.id, epoch, foreground));
    let _pointer_feedback = super::super::computer_pointer_feedback::PointerFeedback::begin(
        current.id,
        !matches!(
            effective_action,
            ControlAction::Invoke | ControlAction::SetValue
        ),
    );
    commit_tracker.result(commit_tracker.ensure_running())?;
    let summary = match effective_action {
        ControlAction::FocusWindow => {
            route = "window_focus";
            commit_tracker.result_as(
                PreCommitFailureKind::FocusUnavailable,
                focus_window(&current, commit_tracker),
            )?;
            current = commit_tracker.result(current_window(&current))?.1;
            format!("Focused and restored window {}.", current.id)
        }
        ControlAction::Invoke => {
            delivery = "background";
            let element_id = args.element_id.as_deref().expect("validated element_id");
            let expected =
                before_control_commit(super::semantic_element(observed, element_id, "invoke"))?;
            let live = before_control_commit_as(
                PreCommitFailureKind::ObservationStale,
                resolve_live_element(&current, observed, expected),
            )?;
            let bounds = &live.snapshot.screen_bounds;
            let point = (
                bounds.x + (bounds.width / 2) as i32,
                bounds.y + (bounds.height / 2) as i32,
            );
            commit_tracker.result(approach_pointer(
                point,
                &current,
                commit_tracker,
                "approach semantic target",
                PointerApproach::FeedbackOnly,
            ))?;
            let live = commit_tracker.result_as(
                PreCommitFailureKind::ObservationStale,
                resolve_live_element(&current, observed, expected),
            )?;
            commit_tracker.result(commit_tracker.ensure_running())?;
            route = invoke_element(&live.element, &current, commit_tracker)?;
            super::super::computer_pointer_feedback::pulse_at(point, || {
                commit_tracker.ensure_running().is_ok()
            });
            format!(
                "Invoked semantic element {element_id} in window {}.",
                current.id
            )
        }
        ControlAction::SetValue => {
            delivery = "background";
            let element_id = args.element_id.as_deref().expect("validated element_id");
            let expected =
                before_control_commit(super::semantic_element(observed, element_id, "set_value"))?;
            let live = before_control_commit_as(
                PreCommitFailureKind::ObservationStale,
                resolve_live_element(&current, observed, expected),
            )?;
            let text = args.text.as_deref().expect("validated text");
            let bounds = &live.snapshot.screen_bounds;
            super::super::computer_pointer_feedback::show_at((
                bounds.x + (bounds.width / 2) as i32,
                bounds.y + (bounds.height / 2) as i32,
            ));
            route = set_element_value(
                &live.element,
                &live.snapshot,
                &current,
                text,
                commit_tracker,
            )?;
            format!(
                "Set {} character(s) on semantic element {element_id} in window {}.",
                text.chars().count(),
                current.id
            )
        }
        ControlAction::MoveMouse => {
            commit_tracker.result_as(
                PreCommitFailureKind::FocusUnavailable,
                focus_window(&current, commit_tracker),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::ObservationStale,
                ensure_observation_fresh_after_focus(observed, &current),
            )?;
            let target = commit_tracker.result_as(
                PreCommitFailureKind::CoordinateTransform,
                resolve_pointer_target(
                    args.element_id.as_deref(),
                    args.x,
                    args.y,
                    coordinate_space,
                    observed,
                    &current,
                    "move_mouse",
                ),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::ObservationStale,
                ensure_target_patch_fresh(observed, &current, target.point),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::UserTakeover,
                ensure_target_foreground(&current),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::TargetOccluded,
                ensure_point_targets_window(target.point, &current),
            )?;
            commit_tracker.result(commit_tracker.mark())?;
            commit_tracker.result(approach_pointer(
                target.point,
                &current,
                commit_tracker,
                "move mouse",
                PointerApproach::Foreground,
            ))?;
            commit_tracker.result_as(
                PreCommitFailureKind::TargetOccluded,
                ensure_point_targets_window(target.point, &current),
            )?;
            if let Some(live) = target.live.as_ref() {
                commit_tracker.result(ensure_point_matches_live_element(live, target.point))?;
            }
            format!("Moved the cursor inside window {}.", current.id)
        }
        ControlAction::Click => {
            commit_tracker.result_as(
                PreCommitFailureKind::FocusUnavailable,
                focus_window(&current, commit_tracker),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::ObservationStale,
                ensure_observation_fresh_after_focus(observed, &current),
            )?;
            let target = commit_tracker.result_as(
                PreCommitFailureKind::CoordinateTransform,
                resolve_pointer_target(
                    args.element_id.as_deref(),
                    args.x,
                    args.y,
                    coordinate_space,
                    observed,
                    &current,
                    "click",
                ),
            )?;
            let button = commit_tracker.result(mouse_button(args.button.as_deref()))?;
            let count = args.click_count.unwrap_or(1);
            commit_tracker.result_as(
                PreCommitFailureKind::ObservationStale,
                ensure_target_patch_fresh(observed, &current, target.point),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::UserTakeover,
                ensure_target_foreground(&current),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::UserTakeover,
                ensure_mouse_button_not_physically_pressed(button),
            )?;
            commit_tracker.result(commit_tracker.mark())?;
            commit_tracker.result(approach_pointer(
                target.point,
                &current,
                commit_tracker,
                "move mouse before click",
                PointerApproach::Foreground,
            ))?;
            thread::sleep(INPUT_SETTLE);
            if target.live.is_none() {
                commit_tracker.result_as(
                    PreCommitFailureKind::ObservationStale,
                    ensure_target_patch_fresh(observed, &current, target.point),
                )?;
            }
            commit_tracker.result(commit_tracker.ensure_running())?;
            commit_tracker.result_as(
                PreCommitFailureKind::UserTakeover,
                ensure_target_foreground(&current),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::UserTakeover,
                ensure_cursor_at(target.point, "click preparation"),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::TargetOccluded,
                ensure_point_targets_window(target.point, &current),
            )?;
            if let Some(live) = target.live.as_ref() {
                commit_tracker.result(ensure_point_matches_live_element(live, target.point))?;
            }
            for index in 0..count {
                commit_tracker.result(commit_tracker.ensure_running())?;
                if let Err(error) = ensure_mouse_button_not_physically_pressed(button) {
                    if index > 0 {
                        return Err(ControlFailure::effect_may_have_occurred(
                            CoreError::Internal(format!(
                            "User mouse input appeared after a partial multi-click; action effect is uncertain: {error}"
                        ))));
                    }
                    return Err(ControlFailure::effect_may_have_occurred(error));
                }
                if let Err(error) = ensure_point_targets_window(target.point, &current) {
                    if index > 0 {
                        return Err(ControlFailure::effect_may_have_occurred(
                            CoreError::Internal(format!(
                            "Another surface appeared after a partial multi-click; action effect is uncertain: {error}"
                        ))));
                    }
                    return Err(ControlFailure::effect_may_have_occurred(error));
                }
                commit_tracker.result(commit_tracker.mark())?;
                commit_tracker.result(send_mouse_click(button))?;
                if index + 1 < count {
                    thread::sleep(Duration::from_millis(75));
                    commit_tracker.result_as(PreCommitFailureKind::UserTakeover, ensure_target_foreground(&current).map_err(|error| {
                        CoreError::Internal(format!(
                            "Focus changed after a partial multi-click; action effect is uncertain: {error}"
                        ))
                    }))?;
                    commit_tracker.result_as(PreCommitFailureKind::UserTakeover, ensure_cursor_at(target.point, "multi-click").map_err(|error| {
                        CoreError::Internal(format!(
                            "Cursor moved after a partial multi-click; action effect is uncertain: {error}"
                        ))
                    }))?;
                }
            }
            super::super::computer_pointer_feedback::pulse_at(target.point, || {
                commit_tracker.ensure_running().is_ok()
            });
            format!("Clicked {count} time(s) inside window {}.", current.id)
        }
        ControlAction::Drag => {
            commit_tracker.result_as(
                PreCommitFailureKind::FocusUnavailable,
                focus_window(&current, commit_tracker),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::ObservationStale,
                ensure_observation_fresh_after_focus(observed, &current),
            )?;
            let from = commit_tracker.result_as(
                PreCommitFailureKind::CoordinateTransform,
                resolve_pointer_target(
                    args.element_id.as_deref(),
                    args.x,
                    args.y,
                    coordinate_space,
                    observed,
                    &current,
                    "drag source",
                ),
            )?;
            let to = commit_tracker.result_as(
                PreCommitFailureKind::CoordinateTransform,
                resolve_pointer_target(
                    args.to_element_id.as_deref(),
                    args.to_x,
                    args.to_y,
                    coordinate_space,
                    observed,
                    &current,
                    "drag destination",
                ),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::ObservationStale,
                ensure_target_patch_fresh(observed, &current, from.point),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::ObservationStale,
                ensure_target_patch_fresh(observed, &current, to.point),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::UserTakeover,
                ensure_target_foreground(&current),
            )?;
            commit_tracker.result(commit_tracker.mark())?;
            commit_tracker.result(approach_pointer(
                from.point,
                &current,
                commit_tracker,
                "move to drag start",
                PointerApproach::Foreground,
            ))?;
            thread::sleep(INPUT_SETTLE);
            for target in [&from, &to] {
                if target.live.is_none() {
                    commit_tracker.result_as(
                        PreCommitFailureKind::ObservationStale,
                        ensure_target_patch_fresh(observed, &current, target.point),
                    )?;
                }
            }
            commit_tracker.result(drag_mouse(
                from.point,
                to.point,
                &current,
                from.live.as_ref(),
                to.live.as_ref(),
                args.drag_duration_ms.unwrap_or(400),
                commit_tracker,
            ))?;
            format!("Dragged inside window {}.", current.id)
        }
        ControlAction::Scroll => {
            commit_tracker.result_as(
                PreCommitFailureKind::FocusUnavailable,
                focus_window(&current, commit_tracker),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::ObservationStale,
                ensure_observation_fresh_after_focus(observed, &current),
            )?;
            let target = commit_tracker.result_as(
                PreCommitFailureKind::CoordinateTransform,
                resolve_pointer_target(
                    args.element_id.as_deref(),
                    args.x,
                    args.y,
                    coordinate_space,
                    observed,
                    &current,
                    "scroll",
                ),
            )?;
            let scroll_x = args.scroll_x.unwrap_or(0);
            let scroll_y = args.scroll_y.unwrap_or(0);
            commit_tracker.result_as(
                PreCommitFailureKind::ObservationStale,
                ensure_target_patch_fresh(observed, &current, target.point),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::UserTakeover,
                ensure_target_foreground(&current),
            )?;
            commit_tracker.result(commit_tracker.mark())?;
            commit_tracker.result(approach_pointer(
                target.point,
                &current,
                commit_tracker,
                "move mouse before scroll",
                PointerApproach::Foreground,
            ))?;
            thread::sleep(INPUT_SETTLE);
            if target.live.is_none() {
                commit_tracker.result_as(
                    PreCommitFailureKind::ObservationStale,
                    ensure_target_patch_fresh(observed, &current, target.point),
                )?;
            }
            commit_tracker.result(commit_tracker.ensure_running())?;
            commit_tracker.result_as(
                PreCommitFailureKind::UserTakeover,
                ensure_target_foreground(&current),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::UserTakeover,
                ensure_cursor_at(target.point, "scroll preparation"),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::TargetOccluded,
                ensure_point_targets_window(target.point, &current),
            )?;
            if let Some(live) = target.live.as_ref() {
                commit_tracker.result(ensure_point_matches_live_element(live, target.point))?;
            }
            commit_tracker.result(commit_tracker.ensure_running())?;
            if scroll_y != 0 {
                commit_tracker.result(commit_tracker.mark())?;
                commit_tracker.result(send_scroll_steps(false, scroll_y))?;
            }
            if scroll_x != 0 {
                commit_tracker.result(commit_tracker.ensure_running())?;
                if scroll_y != 0 {
                    commit_tracker.result_as(PreCommitFailureKind::UserTakeover, ensure_target_foreground(&current).map_err(|error| {
                        CoreError::Internal(format!(
                            "Focus changed after partial scrolling; action effect is uncertain: {error}"
                        ))
                    }))?;
                    commit_tracker.result_as(PreCommitFailureKind::UserTakeover, ensure_cursor_at(target.point, "horizontal scroll").map_err(|error| {
                        CoreError::Internal(format!(
                            "Cursor moved after partial scrolling; action effect is uncertain: {error}"
                        ))
                    }))?;
                    commit_tracker.result_as(PreCommitFailureKind::TargetOccluded, ensure_point_targets_window(target.point, &current).map_err(|error| {
                        CoreError::Internal(format!(
                            "Another surface appeared after partial scrolling; action effect is uncertain: {error}"
                        ))
                    }))?;
                    if let Some(live) = target.live.as_ref() {
                        commit_tracker.result(ensure_point_matches_live_element(live, target.point).map_err(|error| {
                            CoreError::Internal(format!(
                                "The semantic scroll target changed after partial scrolling; action effect is uncertain: {error}"
                            ))
                        }))?;
                    }
                }
                commit_tracker.result(commit_tracker.mark())?;
                commit_tracker.result(send_scroll_steps(true, scroll_x))?;
            }
            format!("Scrolled inside window {}.", current.id)
        }
        ControlAction::TypeText => {
            commit_tracker.result_as(
                PreCommitFailureKind::FocusUnavailable,
                focus_window(&current, commit_tracker),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::ObservationStale,
                ensure_observation_fresh_after_focus(observed, &current),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::UserTakeover,
                ensure_target_foreground(&current),
            )?;
            let targeted_element = if let Some(element_id) = args.element_id.as_deref() {
                let expected = commit_tracker.result(super::semantic_element(
                    observed,
                    element_id,
                    "type_text",
                ))?;
                let live =
                    commit_tracker.result(resolve_live_element(&current, observed, expected))?;
                let password = unsafe { live.element.CurrentIsPassword() }.map_err(|error| {
                    platform_error("verify target password state before text input", error)
                });
                let password = commit_tracker.result(password)?;
                if live.snapshot.password || password.as_bool() {
                    return Err(commit_tracker.failure_as(
                        PreCommitFailureKind::Refused,
                        invalid("Password elements are protected from text input."),
                    ));
                }
                let bounds = &live.snapshot.screen_bounds;
                super::super::computer_pointer_feedback::show_at((
                    bounds.x + (bounds.width / 2) as i32,
                    bounds.y + (bounds.height / 2) as i32,
                ));
                commit_tracker.result(commit_tracker.mark())?;
                commit_tracker.result(
                    unsafe { live.element.SetFocus() }
                        .map_err(|error| platform_error("focus semantic text target", error)),
                )?;
                thread::sleep(INPUT_SETTLE);
                Some(live)
            } else {
                None
            };
            commit_tracker.result_as(
                PreCommitFailureKind::Refused,
                ensure_focused_target_is_not_password(&current),
            )?;
            let text = args.text.as_deref().expect("validated text");
            commit_tracker.result(commit_tracker.mark())?;
            commit_tracker.result(type_text_checked(text, &current, targeted_element.as_ref()))?;
            drop(targeted_element);
            format!(
                "Typed {} character(s) into window {}.",
                text.chars().count(),
                current.id
            )
        }
        ControlAction::Key => {
            commit_tracker.result_as(
                PreCommitFailureKind::FocusUnavailable,
                focus_window(&current, commit_tracker),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::ObservationStale,
                ensure_observation_fresh_after_focus(observed, &current),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::UserTakeover,
                ensure_target_foreground(&current),
            )?;
            commit_tracker.result_as(
                PreCommitFailureKind::Refused,
                ensure_focused_target_is_not_password(&current),
            )?;
            let sequence = args
                .key_sequence
                .as_deref()
                .expect("validated key_sequence");
            commit_tracker.result(commit_tracker.mark())?;
            commit_tracker.result(send_key_sequence(sequence))?;
            format!("Sent an approved key sequence to window {}.", current.id)
        }
    };

    let cursor_position = if delivery == "foreground" {
        cursor_position().ok()
    } else {
        None
    };
    commit_tracker.progress(crate::observed_interaction::InteractionPhase::Waiting);
    let settle_started = std::time::Instant::now();
    let deadline = settle_started + POST_ACTION_STATE_BUDGET;
    let mut last_snapshot = current.clone();
    let mut latest_signature: Option<Vec<u8>> = None;
    let mut previous_signature: Option<Vec<u8>> = None;
    let mut stable_samples = 0_u8;
    let mut sampled_frames = 0_u16;
    while std::time::Instant::now() < deadline {
        if let Ok(capture) = capture_window(&last_snapshot, CaptureOptions::pixels_only()) {
            sampled_frames = sampled_frames.saturating_add(1);
            last_snapshot = capture.snapshot;
            if let Some(signature) = screenshot_signature(&capture.png) {
                if previous_signature
                    .as_deref()
                    .is_some_and(|previous| screenshot_signatures_match(previous, &signature))
                {
                    stable_samples = stable_samples.saturating_add(1);
                } else {
                    stable_samples = 0;
                }
                previous_signature = Some(signature.clone());
                latest_signature = Some(signature);
            }
            let materially_changed = pre_action_signature
                .as_deref()
                .zip(latest_signature.as_deref())
                .is_some_and(|(before, after)| !screenshot_signatures_match(before, after));
            if stable_samples >= 2
                && (materially_changed || settle_started.elapsed() >= Duration::from_millis(200))
            {
                break;
            }
        }
        thread::sleep(POST_ACTION_SAMPLE_INTERVAL);
    }

    let final_capture_result = capture_window(&last_snapshot, final_capture_options);
    let (capture, observation_error) = match final_capture_result {
        Ok(capture) => (Some(capture), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let window_closed = capture.is_none() && !unsafe { IsWindow(Some(hwnd(current.id))).as_bool() };
    let modal_owner_handoff = window_closed
        .then(|| recovered_modal_owner(&observed.snapshot))
        .flatten();
    let after_signature = capture
        .as_ref()
        .and_then(|capture| screenshot_signature(&capture.png))
        .or(latest_signature);
    let difference = pre_action_signature
        .as_deref()
        .zip(after_signature.as_deref())
        .and_then(|(before, after)| screenshot_difference(before, after));
    let semantic_changed = capture.as_ref().is_some_and(|capture| {
        capture.semantic_enabled
            && capture.semantic_error.is_none()
            && !observed.elements.is_empty()
            && observed.elements != capture.elements
    });
    let state_changed = window_closed
        || semantic_changed
        || difference.is_some_and(|difference| difference.materially_changed);
    let after_hash = capture
        .as_ref()
        .map(|capture| blake3::hash(&capture.png).to_hex().to_string());
    let effect = if modal_owner_handoff.is_some() {
        "modal_owner_handoff"
    } else if window_closed {
        "window_closed"
    } else if state_changed {
        "observed_change"
    } else if capture.is_some() {
        "delivered_unverified"
    } else {
        "unverifiable"
    };
    Ok(ControlOutcome {
        summary,
        capture,
        observation_error,
        cursor_position,
        target_verified: true,
        state_changed,
        verification: VisualVerification {
            before_hash: pre_action_hash,
            after_hash,
            difference,
            stable_samples,
            sampled_frames,
            elapsed_ms: settle_started
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
        },
        route,
        delivery,
        effect,
        window_closed,
        modal_owner_handoff,
    })
}

#[cfg(test)]
mod native_input_tests {
    use super::*;

    #[test]
    fn captured_frames_fit_the_model_image_dimension_envelope() {
        let image = RgbaImage::new(2_000, 1_000);
        let (_png, image_width, image_height, native_width, native_height) =
            resized_png(image).expect("bounded capture");

        assert_eq!(MAX_CAPTURE_EDGE, crate::media::MAX_LLM_IMAGE_DIMENSION);
        assert_eq!((native_width, native_height), (2_000, 1_000));
        assert_eq!(
            (image_width, image_height),
            (
                crate::media::MAX_LLM_IMAGE_DIMENSION,
                crate::media::MAX_LLM_IMAGE_DIMENSION / 2,
            )
        );
    }

    #[test]
    fn model_image_pixel_endpoints_map_to_native_window_endpoints() {
        let model_width = crate::media::MAX_LLM_IMAGE_DIMENSION;
        let model_height = model_width / 2;
        let current = WindowSnapshot {
            input_epoch: 0,
            id: 42,
            pid: 7,
            process_started_at_100ns: 123,
            executable_path_hash: "exe-hash".to_string(),
            window_class: "EditorWindow".to_string(),
            session_id: 1,
            app_name: "Editor".to_string(),
            title: "Document".to_string(),
            x: 100,
            y: 200,
            width: model_width * 2,
            height: model_height * 2,
            minimized: false,
            maximized: false,
            focused: true,
            modal_owner: None,
        };
        let observed = ObservedWindow {
            snapshot: current.clone(),
            image_width: Some(model_width),
            image_height: Some(model_height),
            native_image_width: Some(current.width),
            native_image_height: Some(current.height),
            screenshot_signature: None,
            screenshot_guard: None,
            elements: Vec::new(),
        };

        let point = image_point(
            Some(f64::from(model_width - 1)),
            Some(f64::from(model_height - 1)),
            CoordinateSpace::CapturedImagePixels,
            &observed,
            &current,
            "click",
        )
        .expect("coordinate mapping");

        assert_eq!(
            point,
            (
                current.x + current.width as i32 - 1,
                current.y + current.height as i32 - 1,
            )
        );
    }

    #[test]
    fn text_input_normalizes_crlf_and_never_duplicates_control_characters() {
        let operations = text_input_operations("a\r\nb\n\tc");
        assert_eq!(operations.len(), 6);
        assert!(matches!(&operations[0], TextInputOperation::Text(value) if value == "a"));
        assert!(matches!(&operations[1], TextInputOperation::Key(key) if *key == VK_RETURN));
        assert!(matches!(&operations[2], TextInputOperation::Text(value) if value == "b"));
        assert!(matches!(&operations[3], TextInputOperation::Key(key) if *key == VK_RETURN));
        assert!(matches!(&operations[4], TextInputOperation::Key(key) if *key == VK_TAB));
        assert!(matches!(&operations[5], TextInputOperation::Text(value) if value == "c"));
    }

    #[test]
    fn navigation_keys_use_extended_key_flags_on_press_and_release() {
        let down = virtual_key_input(VK_DELETE, false);
        let up = virtual_key_input(VK_DELETE, true);
        let down_flags = unsafe { down.Anonymous.ki.dwFlags };
        let up_flags = unsafe { up.Anonymous.ki.dwFlags };
        assert!(down_flags.contains(KEYEVENTF_EXTENDEDKEY));
        assert!(up_flags.contains(KEYEVENTF_EXTENDEDKEY));
        assert!(up_flags.contains(KEYEVENTF_KEYUP));
    }
}
