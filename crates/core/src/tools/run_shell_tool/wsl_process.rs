//! Windows Job Objects do not own Linux descendants of wsl.exe.
//! Selected WSL commands use a separate Linux process group and a bounded
//! cleanup call, without terminating the distribution or unrelated processes.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub(super) struct WslProcessLease {
    program: String,
    distribution: String,
    directory: String,
    stopped: AtomicBool,
    cleanup: Arc<Mutex<Option<bool>>>,
}

const SUPERVISOR: &str = r#"lease=$1; shift
umask 077
mkdir -m 700 "$lease" || exit 125
printf '%s\n' "$$" > "$lease/pid"
trap 'trap "" TERM; kill -TERM -- -$$ 2>/dev/null || true' EXIT
"$@"
exit $?"#;

const CLEANUP: &str = r#"lease=$1
for attempt in $(seq 1 30); do
  if [ -f "$lease/pid" ]; then
    read -r pid < "$lease/pid"
    case "$pid" in ''|*[!0-9]*) exit 1;; esac
    [ "$pid" -gt 1 ] || exit 1
    kill -KILL -- "-$pid" 2>/dev/null || true
    rm -f -- "$lease/pid"
    rmdir -- "$lease" 2>/dev/null || true
    exit 0
  fi
  sleep 0.1
done
exit 2"#;

pub(super) fn prepare(program: &str, args: &[String]) -> (Vec<String>, Option<WslProcessLease>) {
    if !cfg!(windows)
        || !program.to_ascii_lowercase().ends_with("wsl.exe")
        || args.len() != 8
        || args[0] != "--distribution"
        || args[2] != "--cd"
        || args[4..7] != ["--exec", "bash", "-lc"]
    {
        return (args.to_vec(), None);
    }
    let directory = format!("/tmp/nexa-shell-{}", uuid::Uuid::new_v4());
    let mut wrapped = args[..5].to_vec();
    wrapped.extend([
        "setsid".into(),
        "--wait".into(),
        "bash".into(),
        "-c".into(),
        SUPERVISOR.into(),
        "nexa-shell".into(),
        directory.clone(),
        "bash".into(),
        "-lc".into(),
        args[7].clone(),
    ]);
    (
        wrapped,
        Some(WslProcessLease {
            program: program.into(),
            distribution: args[1].clone(),
            directory,
            stopped: AtomicBool::new(false),
            cleanup: Arc::new(Mutex::new(None)),
        }),
    )
}

impl WslProcessLease {
    pub(super) fn terminate(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        let program = self.program.clone();
        let distribution = self.distribution.clone();
        let directory = self.directory.clone();
        let cleanup = self.cleanup.clone();
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

    pub(super) async fn wait_for_cleanup(&self) -> Result<(), String> {
        self.terminate();
        let start = std::time::Instant::now();
        loop {
            let completed = *self.cleanup.lock().unwrap_or_else(|e| e.into_inner());
            match completed {
                Some(true) => return Ok(()),
                Some(false) => return Err("WSL process cleanup could not be confirmed; inspect the selected distribution before retrying.".into()),
                None if start.elapsed() > std::time::Duration::from_secs(6) => return Err("WSL process cleanup timed out; its state is uncertain.".into()),
                None => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
            }
        }
    }
}

impl Drop for WslProcessLease {
    fn drop(&mut self) {
        self.terminate();
    }
}
