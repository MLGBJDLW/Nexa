//! Physical input provenance for Computer Use. No key text is retained.
//! Hook callbacks only publish counters; cancellation and receipts stay with
//! the owning action, outside the Windows input thread.
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

static PHYSICAL: AtomicU64 = AtomicU64::new(0);
static ESCAPE: AtomicU64 = AtomicU64::new(0);
static WINDOWS: OnceLock<Mutex<WindowRevisions>> = OnceLock::new();
#[cfg(test)]
static INJECTED_FIXTURE_EVENTS: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
struct WindowRevisions {
    clock: u64,
    entries: HashMap<u64, (u64, u64)>,
}
impl WindowRevisions {
    fn observe(&mut self, window: u64) -> u64 {
        self.clock = self.clock.saturating_add(1);
        if let Some((revision, accessed)) = self.entries.get_mut(&window) {
            *accessed = self.clock;
            return *revision;
        }
        if self.entries.len() >= 1024 {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, accessed))| accessed)
                .map(|(id, _)| *id)
            {
                self.entries.remove(&oldest);
            }
        }
        // Re-registering an evicted window changes its revision, so an old
        // observation never becomes valid again after lost monitoring history.
        self.entries.insert(window, (self.clock, self.clock));
        self.clock
    }
    #[cfg(any(all(windows, feature = "desktop-control"), test))]
    fn record(&mut self, window: u64) {
        if let Some((revision, accessed)) = self.entries.get_mut(&window) {
            self.clock = self.clock.saturating_add(1);
            *revision = self.clock;
            *accessed = self.clock;
        }
    }
}
fn windows() -> &'static Mutex<WindowRevisions> {
    WINDOWS.get_or_init(|| Mutex::new(WindowRevisions::default()))
}

#[cfg(any(all(windows, feature = "desktop-control"), test))]
pub(super) fn record(window: u64, injected: bool, escape: bool) {
    record_event(window, injected, escape, true);
}

#[cfg(any(all(windows, feature = "desktop-control"), test))]
fn record_event(window: u64, injected: bool, escape: bool, affects_window: bool) {
    if injected {
        #[cfg(test)]
        INJECTED_FIXTURE_EVENTS.fetch_add(1, Ordering::AcqRel);
        return;
    }
    PHYSICAL.fetch_add(1, Ordering::AcqRel);
    if affects_window {
        windows()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .record(window);
    }
    if escape {
        ESCAPE.fetch_add(1, Ordering::AcqRel);
    }
}

/// Availability is part of the revision: a recovered monitor invalidates
/// observations taken during a gap instead of treating an unchanged zero as proof.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct InputEpoch {
    generation: u64,
    revision: u64,
    monitored: bool,
}

impl InputEpoch {
    pub(super) fn same_monitor(self, other: Self) -> bool {
        self.generation == other.generation && self.monitored == other.monitored
    }
    pub(super) fn capabilities(self) -> serde_json::Value {
        serde_json::json!({
            "physicalInputMonitoring": if self.monitored { "available" } else { "unavailable" },
            "physicalEscapeStop": self.monitored,
            "instantaneousInputGuards": true,
            "stopControlAvailable": true,
        })
    }
}

pub(super) fn window_epoch(window: u64) -> InputEpoch {
    let mut epoch = monitor_epoch();
    epoch.revision = windows()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .observe(window);
    epoch
}
pub(super) fn physical_epoch() -> InputEpoch {
    let mut epoch = monitor_epoch();
    epoch.revision = PHYSICAL.load(Ordering::Acquire);
    epoch
}
pub(super) fn escape_epoch() -> u64 {
    // Also refresh monitor liveness while the action is active.
    let _ = monitor_epoch();
    ESCAPE.load(Ordering::Acquire)
}

#[cfg(not(all(windows, feature = "desktop-control")))]
fn monitor_epoch() -> InputEpoch {
    InputEpoch::default()
}

#[cfg(any(all(windows, feature = "desktop-control"), test))]
#[derive(Default)]
struct MonitorHealth {
    generation: u64,
    available: bool,
    last_attempt: Option<std::time::Instant>,
}

#[cfg(any(all(windows, feature = "desktop-control"), test))]
impl MonitorHealth {
    fn can_retry(&self, now: std::time::Instant, worker_running: bool) -> bool {
        !worker_running
            && self
                .last_attempt
                .is_none_or(|last| now.duration_since(last) >= std::time::Duration::from_secs(1))
    }
    fn observe(&mut self, ready: bool) -> InputEpoch {
        if self.available != ready {
            self.generation = self.generation.wrapping_add(1);
            self.available = ready;
        }
        InputEpoch {
            generation: self.generation,
            monitored: ready,
            revision: 0,
        }
    }
}

#[cfg(all(windows, feature = "desktop-control"))]
fn monitor_epoch() -> InputEpoch {
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant};
    struct Worker {
        thread: std::thread::JoinHandle<()>,
        ready: Arc<std::sync::atomic::AtomicBool>,
        stop: Arc<std::sync::atomic::AtomicBool>,
        heartbeat: Arc<AtomicU64>,
        started: Instant,
    }
    #[derive(Default)]
    struct Monitor {
        worker: Option<Worker>,
        health: MonitorHealth,
    }
    static MONITOR: OnceLock<Mutex<Monitor>> = OnceLock::new();
    let mut monitor = MONITOR
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let running = monitor
        .worker
        .as_ref()
        .is_some_and(|worker| !worker.thread.is_finished());
    if monitor.health.can_retry(now, running) {
        // A replacement thread must invalidate the old generation even if it
        // becomes ready before this caller samples its status.
        monitor.health.observe(false);
        monitor.health.last_attempt = Some(now);
        let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let heartbeat = Arc::new(AtomicU64::new(0));
        let (worker_ready, worker_stop, worker_heartbeat) =
            (ready.clone(), stop.clone(), heartbeat.clone());
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawned = std::thread::Builder::new()
            .name("nexa-physical-input".into())
            .spawn(move || {
                use windows::Win32::System::LibraryLoader::GetModuleHandleW;
                use windows::Win32::UI::WindowsAndMessaging::*;
                let result = unsafe {
                    (|| -> windows::core::Result<_> {
                        let module = GetModuleHandleW(None)?;
                        let keyboard = SetWindowsHookExW(
                            WH_KEYBOARD_LL,
                            Some(keyboard_hook),
                            Some(module.into()),
                            0,
                        )?;
                        match SetWindowsHookExW(
                            WH_MOUSE_LL,
                            Some(mouse_hook),
                            Some(module.into()),
                            0,
                        ) {
                            Ok(mouse) => Ok((keyboard, mouse)),
                            Err(error) => {
                                let _ = UnhookWindowsHookEx(keyboard);
                                Err(error)
                            }
                        }
                    })()
                };
                let hooks = match result {
                    Ok(hooks) => hooks,
                    Err(error) => {
                        let _ = tx.send(Err(error.to_string()));
                        return;
                    }
                };
                unsafe {
                    let timer = SetTimer(None, 0, 250, None);
                    if timer != 0 && !worker_stop.load(Ordering::Acquire) {
                        worker_heartbeat.store(now.elapsed().as_millis() as u64, Ordering::Release);
                        worker_ready.store(true, Ordering::Release);
                        let _ = tx.send(Ok(()));
                        let mut message = MSG::default();
                        while !worker_stop.load(Ordering::Acquire)
                            && GetMessageW(&mut message, None, 0, 0).0 > 0
                        {
                            worker_heartbeat
                                .store(now.elapsed().as_millis() as u64, Ordering::Release);
                            let _ = TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    } else {
                        let _ = tx.send(Err("Input monitor timer/startup unavailable".into()));
                    }
                    worker_ready.store(false, Ordering::Release);
                    if timer != 0 {
                        let _ = KillTimer(None, timer);
                    }
                    let _ = UnhookWindowsHookEx(hooks.0);
                    let _ = UnhookWindowsHookEx(hooks.1);
                }
            });
        let startup = match spawned {
            Ok(thread) => {
                monitor.worker = Some(Worker {
                    thread,
                    ready,
                    stop: stop.clone(),
                    heartbeat,
                    started: now,
                });
                rx.recv_timeout(Duration::from_secs(2))
                    .map_err(|e| e.to_string())
                    .and_then(|value| value)
            }
            Err(error) => {
                monitor.worker = None;
                Err(error.to_string())
            }
        };
        if let Err(error) = startup {
            stop.store(true, Ordering::Release);
            tracing::warn!(%error, "Physical input monitor unavailable; retry is enabled and instantaneous input guards remain active");
        }
    }
    let available = monitor.worker.as_ref().is_some_and(|worker| {
        !worker.thread.is_finished()
            && !worker.stop.load(Ordering::Acquire)
            && worker.ready.load(Ordering::Acquire)
            && (worker.started.elapsed().as_millis() as u64)
                .saturating_sub(worker.heartbeat.load(Ordering::Acquire))
                < 2000
    });
    monitor.health.observe(available)
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
        // Pointer motion can affect a foreground input lane, but merely passing
        // over a window is not an edit/click and must not revoke its readback.
        record_event(
            window.0 as u64,
            event.flags & LLMHF_INJECTED != 0,
            false,
            wparam.0 as u32 != WM_MOUSEMOVE,
        );
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_windows_never_share_input_revisions_and_eviction_never_revalidates() {
        let mut revisions = WindowRevisions::default();
        let first = 0x10010;
        let colliding_old_bucket = 0x11000;
        let observed = revisions.observe(first);
        revisions.observe(colliding_old_bucket);
        revisions.record(colliding_old_bucket);
        assert_eq!(revisions.observe(first), observed);
        revisions.record(first);
        assert_ne!(revisions.observe(first), observed);
        let current = revisions.observe(first);
        for window in 1..=1025 {
            revisions.observe(window);
        }
        assert!(revisions.entries.len() <= 1024);
        assert_ne!(revisions.observe(first), current);
    }

    #[test]
    fn pointer_motion_does_not_revoke_a_window_observation() {
        let window = 0x6f1010;
        let before = window_epoch(window);
        record_event(window, false, false, false);
        assert_eq!(window_epoch(window), before);
        record_event(window, false, false, true);
        assert_ne!(window_epoch(window), before);
    }
    #[test]
    fn failed_monitor_retries_and_recovery_invalidates_gap_observations() {
        let now = std::time::Instant::now();
        let mut health = MonitorHealth::default();
        assert!(health.can_retry(now, false));
        health.last_attempt = Some(now);
        let gap = health.observe(false);
        assert_eq!(gap.capabilities()["physicalInputMonitoring"], "unavailable");
        assert!(!health.can_retry(now, false));
        assert!(health.can_retry(now + std::time::Duration::from_secs(1), false));
        let recovered = health.observe(true);
        assert_ne!(gap, recovered);
        assert_eq!(recovered.capabilities()["physicalEscapeStop"], true);
        let dead = health.observe(false);
        assert_ne!(recovered, dead);
        assert_ne!(dead, gap);
        assert!(
            !health.can_retry(now + std::time::Duration::from_secs(3), true),
            "never create a second hook owner while the first worker is alive"
        );
    }

    #[test]
    fn physical_epoch_remembers_released_input_but_ignores_injected_escape() {
        let window = 0x1f30;
        let before = window_epoch(window).revision;
        let escape = ESCAPE.load(Ordering::Acquire);
        record(window, true, true);
        assert_eq!(window_epoch(window).revision, before);
        assert_eq!(ESCAPE.load(Ordering::Acquire), escape);
        record(window, false, false);
        assert!(window_epoch(window).revision > before);
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
