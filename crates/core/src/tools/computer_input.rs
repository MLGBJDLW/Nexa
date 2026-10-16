//! Physical input provenance for Computer Use. No key text is retained.
//! Hook callbacks only publish counters; cancellation and receipts stay with
//! the owning action, outside the Windows input thread.
use std::sync::atomic::{AtomicU64, Ordering};

static PHYSICAL: AtomicU64 = AtomicU64::new(0);
static ESCAPE: AtomicU64 = AtomicU64::new(0);
static WINDOWS: [AtomicU64; 256] = [const { AtomicU64::new(0) }; 256];
#[cfg(test)]
static INJECTED_FIXTURE_EVENTS: AtomicU64 = AtomicU64::new(0);

fn window_slot(window: u64) -> usize {
    ((window >> 4) ^ (window >> 12)) as usize % WINDOWS.len()
}

#[cfg(any(all(windows, feature = "desktop-control"), test))]
pub(super) fn record(window: u64, injected: bool, escape: bool) {
    if injected {
        #[cfg(test)]
        INJECTED_FIXTURE_EVENTS.fetch_add(1, Ordering::AcqRel);
        return;
    }
    PHYSICAL.fetch_add(1, Ordering::AcqRel);
    WINDOWS[window_slot(window)].fetch_add(1, Ordering::AcqRel);
    if escape {
        ESCAPE.fetch_add(1, Ordering::AcqRel);
    }
}

pub(super) fn window_epoch(window: u64) -> u64 {
    start();
    WINDOWS[window_slot(window)].load(Ordering::Acquire)
}
pub(super) fn physical_epoch() -> u64 {
    start();
    PHYSICAL.load(Ordering::Acquire)
}
pub(super) fn escape_epoch() -> u64 {
    start();
    ESCAPE.load(Ordering::Acquire)
}

#[cfg(not(all(windows, feature = "desktop-control")))]
fn start() {}

#[cfg(all(windows, feature = "desktop-control"))]
fn start() {
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    STARTED.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawned = std::thread::Builder::new().name("nexa-physical-input".into()).spawn(move || {
            use windows::Win32::System::LibraryLoader::GetModuleHandleW;
            use windows::Win32::UI::WindowsAndMessaging::*;
            let result = unsafe { (|| -> windows::core::Result<_> {
                let module = GetModuleHandleW(None)?;
                let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(module.into()), 0)?;
                match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(module.into()), 0) {
                    Ok(mouse) => Ok((keyboard, mouse)),
                    Err(error) => { let _ = UnhookWindowsHookEx(keyboard); Err(error) }
                }
            })() };
            let hooks = match result {
                Ok(hooks) => { let _ = tx.send(Ok(())); hooks },
                Err(error) => { let _ = tx.send(Err(error.to_string())); return; }
            };
            let mut message = MSG::default();
            unsafe {
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                let _ = UnhookWindowsHookEx(hooks.0);
                let _ = UnhookWindowsHookEx(hooks.1);
            }
        });
        if let Err(error) = spawned.map_err(|e| e.to_string()).and_then(|_| rx.recv_timeout(std::time::Duration::from_secs(2)).map_err(|e| e.to_string())?) {
            tracing::warn!(%error, "Physical input monitor unavailable; instantaneous input guards remain active");
        }
    });
}

#[cfg(all(windows, feature = "desktop-control"))]
unsafe extern "system" fn keyboard_hook(
    code: i32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::*;
    if code >= 0 {
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        record(
            unsafe { GetForegroundWindow() }.0 as u64,
            event.flags.0 & LLKHF_INJECTED.0 != 0,
            event.vkCode == 0x1b && matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN),
        );
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(all(windows, feature = "desktop-control"))]
unsafe extern "system" fn mouse_hook(
    code: i32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::*;
    if code >= 0 {
        let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let window = unsafe { GetAncestor(WindowFromPoint(event.pt), GA_ROOT) };
        record(window.0 as u64, event.flags & LLMHF_INJECTED != 0, false);
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_epoch_remembers_released_input_but_ignores_injected_escape() {
        let window = 0x1f30;
        let before = WINDOWS[window_slot(window)].load(Ordering::Acquire);
        let escape = ESCAPE.load(Ordering::Acquire);
        record(window, true, true);
        assert_eq!(WINDOWS[window_slot(window)].load(Ordering::Acquire), before);
        assert_eq!(ESCAPE.load(Ordering::Acquire), escape);
        record(window, false, false);
        assert!(WINDOWS[window_slot(window)].load(Ordering::Acquire) > before);
        record(window, false, true);
        assert!(ESCAPE.load(Ordering::Acquire) > escape);
    }

    #[cfg(all(windows, feature = "desktop-control"))]
    #[test]
    #[ignore = "Uses the interactive Windows input desktop; run serially"]
    fn native_hook_observes_sendinput_without_revoking_physical_input_epoch() {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        let before = physical_epoch();
        let injected = INJECTED_FIXTURE_EVENTS.load(Ordering::Acquire);
        // Zero-delta motion can be coalesced away by Windows. Move one pixel
        // out and back so this verifies a real injected event, with no click.
        let inputs = [1, -1].map(|dx| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dwFlags: MOUSEEVENTF_MOVE,
                    ..Default::default()
                },
            },
        });
        assert_eq!(
            unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) },
            2
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while INJECTED_FIXTURE_EVENTS.load(Ordering::Acquire) == injected
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            INJECTED_FIXTURE_EVENTS.load(Ordering::Acquire) > injected,
            "the actual low-level hook must receive injected motion"
        );
        assert_eq!(
            physical_epoch(),
            before,
            "Nexa motion must not look like physical takeover"
        );
    }
}
