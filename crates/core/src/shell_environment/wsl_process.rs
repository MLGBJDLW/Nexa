//! Windows Job Objects do not own Linux descendants of wsl.exe.
//! WSL commands and terminals own Linux sessions and use bounded
//! cleanup call, without terminating the distribution or unrelated processes.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

pub struct WslProcessLease {
    inner: Arc<LeaseState>,
}

struct LeaseState {
    program: String,
    distribution: String,
    directory: String,
    stopped: AtomicBool,
    cleanup: Arc<Mutex<Option<bool>>>,
}

#[derive(Default)]
struct LeaseRegistry {
    closing: bool,
    leases: Vec<Weak<LeaseState>>,
}
fn registry() -> &'static Mutex<LeaseRegistry> {
    static REGISTRY: OnceLock<Mutex<LeaseRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(LeaseRegistry::default()))
}

/// Host shutdown must finish Linux cleanup before Windows exits its proxies.
pub fn shutdown_all_blocking() -> Vec<String> {
    let leases = {
        let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
        registry.closing = true;
        registry
            .leases
            .iter()
            .filter_map(Weak::upgrade)
            .map(|inner| WslProcessLease { inner })
            .collect::<Vec<_>>()
    };
    for lease in &leases {
        lease.terminate();
    }
    leases
        .iter()
        .filter_map(|lease| lease.wait_for_cleanup_blocking().err())
        .collect()
}

const SUPERVISOR: &str = r#"lease=$1; shift
umask 077
mkdir -m 700 "$lease" || exit 125
printf '%s\n' "$$" > "$lease/pid"
trap 'trap "" TERM; kill -TERM -- -$$ 2>/dev/null || true' EXIT
"$@"
exit $?"#;

const INTERACTIVE_SUPERVISOR: &str = r#"lease=$1; shift
[ -t 0 ] || exit 125
read -r record < /proc/$$/stat || exit 125
read -r state parent group session rest <<< "${record##*) }"
case "$session" in ''|*[!0-9]*) exit 125;; esac
[ "$session" -gt 1 ] || exit 125
umask 077
mkdir -m 700 "$lease" || exit 125
printf '%s\n' "$session" > "$lease/pid"
exec "$@""#;

const CLEANUP: &str = r#"lease=$1
for attempt in $(seq 1 30); do
  if [ -f "$lease/pid" ]; then
    read -r pid < "$lease/pid"
    case "$pid" in ''|*[!0-9]*) exit 1;; esac
    [ "$pid" -gt 1 ] || exit 1
    # Interactive job control creates additional process groups. Reap the
    # complete owned Linux session, not only the foreground Bash group.
    for pass in 1 2; do
      for stat in /proc/[0-9]*/stat; do
        IFS= read -r record < "$stat" 2>/dev/null || continue
        read -r state parent group session rest <<< "${record##*) }"
        [ "$session" = "$pid" ] || continue
        process=${stat#/proc/}; process=${process%/stat}
        kill -KILL -- "$process" 2>/dev/null || true
      done
    done
    kill -KILL -- "-$pid" 2>/dev/null || true
    rm -f -- "$lease/pid"
    rmdir -- "$lease" 2>/dev/null || true
    exit 0
  fi
  sleep 0.1
done
exit 2"#;

pub fn prepare(
    program: &str,
    args: &[String],
) -> Result<(Vec<String>, Option<WslProcessLease>), String> {
    let interactive = args.len() == 7 && args[6] == "-il";
    if !cfg!(windows)
        || !program.to_ascii_lowercase().ends_with("wsl.exe")
        || !(args.len() == 8 || interactive)
        || args[0] != "--distribution"
        || args[2] != "--cd"
        || args[4..6] != ["--exec", "bash"]
        || (!interactive && args[6] != "-lc")
    {
        return Ok((args.to_vec(), None));
    }
    let directory = format!("/tmp/nexa-shell-{}", uuid::Uuid::new_v4());
    let mut wrapped = args[..5].to_vec();
    // WSL already owns a separate controlling terminal/session for each
    // ConPTY launch. setsid would detach it, and TIOCSCTTY cannot steal it back.
    // Record that existing session for interactive launches; create a new
    // session only for pipe-backed agent commands.
    if !interactive {
        wrapped.extend(["setsid".into(), "--wait".into()]);
    }
    wrapped.extend([
        "bash".into(),
        "-c".into(),
        if interactive {
            INTERACTIVE_SUPERVISOR.into()
        } else {
            SUPERVISOR.into()
        },
        "nexa-shell".into(),
        directory.clone(),
        "bash".into(),
    ]);
    wrapped.extend_from_slice(&args[6..]);
    let mut registry = registry()
        .lock()
        .map_err(|_| "WSL lease registry is unavailable")?;
    if registry.closing {
        return Err("WSL launches are disabled while Nexa is shutting down".into());
    }
    let inner = Arc::new(LeaseState {
        program: program.into(),
        distribution: args[1].clone(),
        directory,
        stopped: AtomicBool::new(false),
        cleanup: Arc::new(Mutex::new(None)),
    });
    registry.leases.retain(|lease| lease.strong_count() > 0);
    registry.leases.push(Arc::downgrade(&inner));
    Ok((wrapped, Some(WslProcessLease { inner })))
}

impl WslProcessLease {
    pub fn terminate(&self) {
        if self.inner.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        let program = self.inner.program.clone();
        let distribution = self.inner.distribution.clone();
        let directory = self.inner.directory.clone();
        let cleanup = self.inner.cleanup.clone();
        // Never wait for transport cleanup under a runtime registry mutex.
        std::thread::spawn(move || {
            let mut command = std::process::Command::new(program);
            command
                .args([
                    "--distribution",
                    &distribution,
                    "--exec",
                    "bash",
                    "-c",
                    CLEANUP,
                    "nexa-cleanup",
                    &directory,
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            crate::background_process::configure_std_background(&mut command);
            let Ok(mut child) = command.spawn() else {
                *cleanup.lock().unwrap_or_else(|e| e.into_inner()) = Some(false);
                return;
            };
            let start = std::time::Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        *cleanup.lock().unwrap_or_else(|e| e.into_inner()) = Some(status.success());
                        break;
                    }
                    _ if start.elapsed() >= std::time::Duration::from_secs(5) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        *cleanup.lock().unwrap_or_else(|e| e.into_inner()) = Some(false);
                        break;
                    }
                    _ => std::thread::sleep(std::time::Duration::from_millis(25)),
                }
            }
        });
    }

    pub async fn wait_for_cleanup(&self) -> Result<(), String> {
        self.terminate();
        let start = std::time::Instant::now();
        loop {
            let completed = *self.inner.cleanup.lock().unwrap_or_else(|e| e.into_inner());
            match completed {
                Some(true) => return Ok(()),
                Some(false) => return Err("WSL process cleanup could not be confirmed; inspect the selected distribution before retrying.".into()),
                None if start.elapsed() > std::time::Duration::from_secs(6) => return Err("WSL process cleanup timed out; its state is uncertain.".into()),
                None => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
            }
        }
    }

    /// For terminal workers and final host shutdown, after releasing registries.
    pub fn wait_for_cleanup_blocking(&self) -> Result<(), String> {
        self.terminate();
        let start = std::time::Instant::now();
        loop {
            let completed = *self.inner.cleanup.lock().unwrap_or_else(|e| e.into_inner());
            match completed {
                Some(true) => return Ok(()),
                Some(false) => return Err("WSL terminal cleanup could not be confirmed.".into()),
                None if start.elapsed() > std::time::Duration::from_secs(6) => {
                    return Err("WSL terminal cleanup timed out; its state is uncertain.".into())
                }
                None => std::thread::sleep(std::time::Duration::from_millis(25)),
            }
        }
    }
}

impl Drop for WslProcessLease {
    fn drop(&mut self) {
        self.terminate();
    }
}
