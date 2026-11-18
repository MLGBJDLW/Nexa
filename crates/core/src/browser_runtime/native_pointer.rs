use crate::error::CoreError;

/// Process-wide owner for foreground desktop input. Browser native-pointer
/// commits and general computer control must never interleave.
pub fn desktop_input_arbiter() -> &'static std::sync::Arc<tokio::sync::Mutex<()>> {
    static LOCK: std::sync::OnceLock<std::sync::Arc<tokio::sync::Mutex<()>>> =
        std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
}

pub async fn acquire_desktop_input_permit() -> Result<tokio::sync::OwnedMutexGuard<()>, CoreError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        std::sync::Arc::clone(desktop_input_arbiter()).lock_owned(),
    )
    .await
    .map_err(|_| {
        CoreError::InvalidInput(
            "Desktop input is still owned by another action after 5 seconds. No new input was sent; inspect the existing action receipt or restart the desktop input broker before retrying."
                .to_string(),
        )
    })
}

#[cfg(target_os = "windows")]
pub struct CrossProcessDesktopInputGuard {
    handle: windows::Win32::Foundation::HANDLE,
    owned: bool,
}

// The guard owns a Win32 semaphore, not a thread-affine mutex. Windows permits
// ReleaseSemaphore from a different thread, so moving it across Tokio awaits
// is sound.
#[cfg(target_os = "windows")]
unsafe impl Send for CrossProcessDesktopInputGuard {}

#[cfg(target_os = "windows")]
pub fn try_acquire_cross_process_input() -> Result<CrossProcessDesktopInputGuard, CoreError> {
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{CreateSemaphoreW, WaitForSingleObject};

    let handle = unsafe {
        CreateSemaphoreW(
            None,
            1,
            1,
            windows::core::w!("Local\\NexaDesktopInputSemaphoreV3"),
        )
    }
    .map_err(|error| CoreError::Internal(format!("create desktop-input semaphore: {error}")))?;
    let wait = unsafe { WaitForSingleObject(handle, 0) };
    if wait == WAIT_OBJECT_0 {
        return Ok(CrossProcessDesktopInputGuard {
            handle,
            owned: true,
        });
    }
    let _ = unsafe { CloseHandle(handle) };
    if wait == WAIT_TIMEOUT {
        return Err(CoreError::InvalidInput(
            "Another Nexa process is currently using global desktop input. Re-observe after that action finishes."
                .to_string(),
        ));
    }
    Err(CoreError::Internal(
        "Could not acquire the cross-process desktop-input semaphore".to_string(),
    ))
}

#[cfg(target_os = "windows")]
impl Drop for CrossProcessDesktopInputGuard {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::ReleaseSemaphore;

        if self.owned {
            let _ = unsafe { ReleaseSemaphore(self.handle, 1, None) };
            self.owned = false;
        }
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

#[cfg(not(target_os = "windows"))]
pub struct CrossProcessDesktopInputGuard;

#[cfg(not(target_os = "windows"))]
pub fn try_acquire_cross_process_input() -> Result<CrossProcessDesktopInputGuard, CoreError> {
    Err(CoreError::InvalidInput(
        "Cross-process desktop input is unavailable on this platform".to_string(),
    ))
}

/// Encode a physical desktop pixel at its center for Win32 absolute input.
#[cfg(windows)]
pub fn absolute_pointer_input(
    point: (i32, i32),
) -> Result<windows::Win32::UI::Input::KeyboardAndMouse::INPUT, CoreError> {
    use windows::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONULL},
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    };
    let pixel = POINT {
        x: point.0,
        y: point.1,
    };
    if unsafe { MonitorFromPoint(pixel, MONITOR_DEFAULTTONULL) }
        .0
        .is_null()
    {
        return Err(CoreError::InvalidInput(
            "The pointer target is outside every connected display.".into(),
        ));
    }
    let axis = |position: i32, origin, size| {
        let offset = i64::from(position) - i64::from(unsafe { GetSystemMetrics(origin) });
        let span = i64::from(unsafe { GetSystemMetrics(size) });
        if !(0..span).contains(&offset) {
            return Err(CoreError::InvalidInput(
                "The pointer target is outside the current virtual desktop.".into(),
            ));
        }
        Ok(((offset * 65536 + 32768) / span) as i32)
    };
    Ok(INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: axis(point.0, SM_XVIRTUALSCREEN, SM_CXVIRTUALSCREEN)?,
                dy: axis(point.1, SM_YVIRTUALSCREEN, SM_CYVIRTUALSCREEN)?,
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                ..Default::default()
            },
        },
    })
}

/// A foreground gesture must yield if real input arrives or monitoring restarts.
#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
pub struct ForegroundInputWatch(crate::tools::computer_input::InputEpoch);

#[cfg(windows)]
impl ForegroundInputWatch {
    pub fn start() -> Result<Self, CoreError> {
        let epoch = crate::tools::computer_input::physical_epoch();
        if !epoch.is_monitored() {
            return Err(CoreError::InvalidInput(
                "Physical input monitoring is unavailable for the foreground browser gesture."
                    .into(),
            ));
        }
        Ok(Self(epoch))
    }

    pub fn changed(self) -> bool {
        self.0 != crate::tools::computer_input::physical_epoch()
    }
}
