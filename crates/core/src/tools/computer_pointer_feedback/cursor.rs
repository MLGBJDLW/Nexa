//! The hardware cursor is above layered windows. A small, lazily started host
//! owns the temporary cursor scheme and restores it even if Nexa exits/crashes.
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{mpsc, Mutex, MutexGuard, OnceLock},
    time::Duration,
};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS,
};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::{
    CopyIcon, CreateIconIndirect, DestroyCursor, LoadCursorW, SetSystemCursor,
    SystemParametersInfoW, HCURSOR, HICON, ICONINFO, OCR_APPSTARTING, OCR_CROSS, OCR_HAND,
    OCR_IBEAM, OCR_NO, OCR_NORMAL, OCR_SIZEALL, OCR_SIZENESW, OCR_SIZENS, OCR_SIZENWSE, OCR_SIZEWE,
    OCR_UP, OCR_WAIT, SPI_SETCURSORS, SYSTEM_CURSOR_ID, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

const FLAG: &str = "--nexa-pointer-feedback-host";
const READY: &str = "NEXA_POINTER_READY";
const OK: &str = "NEXA_POINTER_OK";
const ERROR: &str = "NEXA_POINTER_ERROR";
static EXECUTABLE: OnceLock<(PathBuf, Vec<String>)> = OnceLock::new();
static HOST: Mutex<Option<Host>> = Mutex::new(None);

#[derive(serde::Serialize, serde::Deserialize)]
struct Request {
    active: bool,
    scale: f64,
    accent: [u8; 3],
}

pub(super) fn process_entry() -> bool {
    if std::env::args().nth(1).as_deref() == Some(FLAG) {
        serve();
        return true;
    }
    if let Ok(executable) = std::env::current_exe() {
        let _ = EXECUTABLE.set((executable, vec![FLAG.into()]));
    }
    false
}

struct Host {
    child: Child,
    input: ChildStdin,
    replies: mpsc::Receiver<String>,
    active: bool,
}

impl Host {
    fn start() -> Option<Self> {
        use std::os::windows::process::CommandExt;
        let (executable, args) = EXECUTABLE.get()?;
        let mut child = Command::new(executable)
            .args(args)
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let input = child.stdin.take()?;
        let output = child.stdout.take()?;
        let (sender, replies) = mpsc::sync_channel(4);
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if matches!(line.as_str(), READY | OK | ERROR) && sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut host = Self {
            child,
            input,
            replies,
            active: false,
        };
        if host
            .replies
            .recv_timeout(Duration::from_secs(2))
            .ok()
            .as_deref()
            != Some(READY)
        {
            host.stop();
            return None;
        }
        Some(host)
    }

    fn request(&mut self, request: Request) -> bool {
        self.active |= request.active;
        let Ok(mut bytes) = serde_json::to_vec(&request) else {
            return false;
        };
        bytes.push(b'\n');
        if self.input.write_all(&bytes).is_err() || self.input.flush().is_err() {
            self.stop();
            return false;
        }
        match self
            .replies
            .recv_timeout(Duration::from_secs(2))
            .ok()
            .as_deref()
        {
            Some(OK) => {}
            Some(ERROR) => {
                self.active = false; // the host already rolled back
                self.stop();
                return false;
            }
            _ => {
                self.stop();
                return false;
            }
        }
        self.active = request.active;
        true
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if self.active {
            // The host's exact snapshots are unavailable only if it failed.
            // Reload the configured Windows scheme, without changing settings.
            if let Ok(_lock) = CursorLock::acquire() {
                restore_configured_scheme();
            }
            self.active = false;
        }
    }
}

pub(super) struct CursorLease(MutexGuard<'static, Option<Host>>);

impl CursorLease {
    pub(super) fn begin(scale: f64, accent: [u8; 3]) -> Option<Self> {
        let mut host = HOST.try_lock().ok()?;
        if host.is_none() {
            *host = Host::start();
        }
        if !host.as_mut()?.request(Request {
            active: true,
            scale,
            accent,
        }) {
            *host = None;
            return None;
        }
        Some(Self(host))
    }
}

impl Drop for CursorLease {
    fn drop(&mut self) {
        if let Some(host) = self.0.as_mut() {
            if !host.request(Request {
                active: false,
                scale: 1.0,
                accent: [0; 3],
            }) {
                *self.0 = None;
            }
        }
    }
}

fn reply(value: &str) -> bool {
    writeln!(std::io::stdout(), "{value}").is_ok() && std::io::stdout().flush().is_ok()
}

fn serve() {
    let (sender, input) = mpsc::sync_channel(4);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(std::io::stdin());
        loop {
            let mut line = String::new();
            use std::io::Read;
            if reader
                .by_ref()
                .take(1024)
                .read_line(&mut line)
                .ok()
                .filter(|length| *length > 0 && *length < 1024)
                .is_none()
            {
                break;
            }
            let Ok(request) = serde_json::from_str::<Request>(&line) else {
                break;
            };
            if sender.send(request).is_err() {
                break;
            }
        }
    });
    let mut cursors = None;
    if !reply(READY) {
        return;
    }
    loop {
        // Input actions are short. Also restore if an otherwise-live parent
        // stalls, without leaving an indefinitely modified system cursor.
        match input.recv_timeout(Duration::from_secs(30)) {
            Ok(request) => {
                drop(cursors.take());
                let success = if request.active {
                    cursors = SystemCursors::install(request.scale, request.accent).ok();
                    cursors.is_some()
                } else {
                    true
                };
                if !reply(if success { OK } else { ERROR }) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                drop(cursors.take());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    // Closing the pipe on normal exit or process death releases the exact
    // snapshots, independently of the lifetime of Nexa's action worker.
}

fn restore_configured_scheme() {
    let _ = unsafe {
        SystemParametersInfoW(
            SPI_SETCURSORS,
            0,
            None,
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
}

struct CursorLock(HANDLE);
impl CursorLock {
    fn acquire() -> windows::core::Result<Self> {
        let handle = unsafe { CreateMutexW(None, false, w!("Local\\NexaComputerUseCursor")) }?;
        let result = unsafe { WaitForSingleObject(handle, 0) };
        if result == WAIT_OBJECT_0 || result == WAIT_ABANDONED {
            if result == WAIT_ABANDONED {
                restore_configured_scheme();
            }
            Ok(Self(handle))
        } else {
            let _ = unsafe { CloseHandle(handle) };
            Err(windows::core::Error::new(
                windows::core::HRESULT(0x80004005u32 as i32),
                "Cursor feedback is already owned",
            ))
        }
    }
}
impl Drop for CursorLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

struct SystemCursors {
    original: Vec<(SYSTEM_CURSOR_ID, HCURSOR)>,
    _lock: CursorLock,
}

impl SystemCursors {
    fn install(scale: f64, accent: [u8; 3]) -> windows::core::Result<Self> {
        let scale = if scale.is_finite() {
            scale.clamp(1.0, 4.0)
        } else {
            1.0
        };
        let mut cursors = Self {
            original: Vec::new(),
            _lock: CursorLock::acquire()?,
        };
        let custom = create_cursor(scale, accent)?;
        let result: windows::core::Result<()> = (|| {
            for id in [
                OCR_NORMAL,
                OCR_IBEAM,
                OCR_HAND,
                OCR_CROSS,
                OCR_WAIT,
                OCR_APPSTARTING,
                OCR_NO,
                OCR_UP,
                OCR_SIZEALL,
                OCR_SIZENESW,
                OCR_SIZENWSE,
                OCR_SIZENS,
                OCR_SIZEWE,
            ] {
                let original = unsafe { LoadCursorW(None, PCWSTR(id.0 as usize as *const u16)) }?;
                let snapshot = unsafe { CopyIcon(HICON(original.0)) }?;
                cursors.original.push((id, HCURSOR(snapshot.0)));
                // SetSystemCursor consumes the supplied handle. Never pass a
                // shared system resource or reuse a handle for another shape.
                let copy = unsafe { CopyIcon(HICON(custom.0)) }?;
                unsafe { SetSystemCursor(HCURSOR(copy.0), id) }?;
            }
            Ok(())
        })();
        let _ = unsafe { DestroyCursor(custom) };
        result?;
        Ok(cursors)
    }
}

impl Drop for SystemCursors {
    fn drop(&mut self) {
        for (id, cursor) in self.original.drain(..) {
            let _ = unsafe { SetSystemCursor(cursor, id) };
        }
    }
}

fn create_cursor(scale: f64, accent: [u8; 3]) -> windows::core::Result<HCURSOR> {
    let (width, height, pixels) = super::pointer_pixels(scale, accent);
    let info = BITMAPINFO {
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
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }?;
    unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len()) };
    let mask = unsafe { CreateBitmap(width, height, 1, 1, None) };
    let cursor = if mask.is_invalid() {
        Err(windows::core::Error::from_thread())
    } else {
        unsafe {
            CreateIconIndirect(&ICONINFO {
                fIcon: false.into(),
                xHotspot: (8.0 * scale).round() as u32,
                yHotspot: (8.0 * scale).round() as u32,
                hbmMask: mask,
                hbmColor: bitmap,
            })
        }
        .map(|icon| HCURSOR(icon.0))
    };
    unsafe {
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteObject(mask.into());
    }
    cursor
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_host() {
        let _ = EXECUTABLE.set((
            std::env::current_exe().unwrap(),
            vec![
                "--exact".into(),
                "tools::computer_pointer_feedback::cursor::tests::cursor_host_entry".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
        ));
    }

    fn snapshot(id: SYSTEM_CURSOR_ID) -> (u32, u32, Vec<u8>) {
        use windows::Win32::Graphics::Gdi::{GetBitmapBits, GetObjectW, BITMAP};
        use windows::Win32::UI::WindowsAndMessaging::GetIconInfo;
        let cursor = unsafe { LoadCursorW(None, PCWSTR(id.0 as usize as *const u16)) }.unwrap();
        let mut icon = ICONINFO::default();
        unsafe { GetIconInfo(HICON(cursor.0), &mut icon) }.unwrap();
        let bitmap = if icon.hbmColor.is_invalid() {
            icon.hbmMask
        } else {
            icon.hbmColor
        };
        let mut info = BITMAP::default();
        assert_ne!(
            unsafe {
                GetObjectW(
                    bitmap.into(),
                    std::mem::size_of::<BITMAP>() as i32,
                    Some((&mut info as *mut BITMAP).cast()),
                )
            },
            0
        );
        let mut pixels = vec![0; (info.bmWidthBytes * info.bmHeight) as usize];
        assert_eq!(
            unsafe { GetBitmapBits(bitmap, pixels.len() as i32, pixels.as_mut_ptr().cast()) },
            pixels.len() as i32
        );
        unsafe {
            let _ = DeleteObject(icon.hbmColor.into());
            let _ = DeleteObject(icon.hbmMask.into());
        }
        (icon.xHotspot, icon.yHotspot, pixels)
    }

    #[test]
    #[ignore = "requires interactive Windows; temporarily changes and restores the cursor, sends no input"]
    fn native_cursor_override_restores_on_release_and_parent_exit() {
        test_host();
        let arrow = snapshot(OCR_NORMAL);
        let text = snapshot(OCR_IBEAM);
        {
            let _lease = CursorLease::begin(1.0, [20, 184, 166]).expect("cursor host handshake");
            assert_ne!(snapshot(OCR_NORMAL), arrow);
            assert_ne!(snapshot(OCR_IBEAM), text);
            assert_eq!(snapshot(OCR_NORMAL), snapshot(OCR_IBEAM));
        }
        assert_eq!(snapshot(OCR_NORMAL), arrow);
        assert_eq!(snapshot(OCR_IBEAM), text);
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tools::computer_pointer_feedback::cursor::tests::cursor_parent_exit_entry",
                "--ignored",
                "--nocapture",
            ])
            .status()
            .unwrap();
        assert!(status.success());
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while snapshot(OCR_NORMAL) != arrow && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            snapshot(OCR_NORMAL),
            arrow,
            "pipe closure must restore even without Rust destructors"
        );
        assert_eq!(snapshot(OCR_IBEAM), text);
    }

    #[test]
    #[ignore = "internal subprocess entry for native cursor crash acceptance"]
    fn cursor_parent_exit_entry() {
        test_host();
        let before = snapshot(OCR_NORMAL);
        let _lease = CursorLease::begin(1.0, [224, 108, 50]).expect("cursor host handshake");
        assert_ne!(snapshot(OCR_NORMAL), before);
        std::process::exit(0); // deliberately bypass Drop
    }

    #[test]
    #[ignore = "internal subprocess entry for native cursor lifecycle acceptance"]
    fn cursor_host_entry() {
        serve();
    }
}
