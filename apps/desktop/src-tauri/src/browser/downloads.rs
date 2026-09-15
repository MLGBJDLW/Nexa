use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::watch;

const MAX_BYTES: u64 = 100 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DownloadResult {
    pub id: String,
    pub state: String,
    pub destination: PathBuf,
    pub bytes: u64,
    pub content_hash: Option<String>,
    pub error: Option<String>,
}

pub(super) struct DownloadJob {
    result: watch::Sender<DownloadResult>,
    staging: PathBuf,
    parent: PathBuf,
    started: Instant,
    cancelled: std::sync::atomic::AtomicBool,
}

impl DownloadJob {
    fn terminal(&self) -> bool {
        matches!(self.result.borrow().state.as_str(), "completed" | "failed")
    }
    fn fail(&self, error: String) {
        self.result.send_modify(|result| {
            if !matches!(result.state.as_str(), "completed" | "failed") {
                result.state = "failed".into();
                result.error = Some(error);
            }
        });
    }
    fn id(&self) -> String {
        self.result.borrow().id.clone()
    }
    fn cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Acquire)
            || self.started.elapsed() >= DOWNLOAD_TIMEOUT
    }
    fn complete(&self) {
        let result = (|| -> Result<(u64, String), String> {
            use std::io::Read;
            if self.cancelled() || self.terminal() {
                return Err("Download was cancelled".into());
            }
            if self
                .parent
                .canonicalize()
                .map_err(|error| error.to_string())?
                != self.parent
            {
                return Err("Download destination directory changed".into());
            }
            let metadata =
                std::fs::symlink_metadata(&self.staging).map_err(|error| error.to_string())?;
            if !metadata.is_file()
                || metadata.len() > MAX_BYTES
                || metadata.len() != self.result.borrow().bytes
            {
                return Err("Download is not a regular file within the 100 MiB limit".into());
            }
            let mut file = std::fs::File::open(&self.staging).map_err(|error| error.to_string())?;
            let mut hash = blake3::Hasher::new();
            let mut buffer = [0_u8; 64 * 1024];
            let mut bytes = 0;
            loop {
                let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
                if count == 0 {
                    break;
                }
                bytes += count as u64;
                if bytes > MAX_BYTES || self.cancelled() {
                    return Err("Download exceeded its size or time limit, or was cancelled".into());
                }
                hash.update(&buffer[..count]);
            }
            if bytes != metadata.len() {
                return Err("Download file changed during verification".into());
            }
            Ok((bytes, hash.finalize().to_hex().to_string()))
        })();
        match result {
            Ok((bytes, hash)) => self.result.send_modify(|result| {
                if matches!(result.state.as_str(), "failed" | "completed") {
                    return;
                }
                if self.cancelled()
                    || self.parent.canonicalize().ok().as_ref() != Some(&self.parent)
                {
                    result.state = "failed".into();
                    result.error = Some(
                        "Download cancelled or destination directory changed before publication"
                            .into(),
                    );
                    return;
                }
                // Same-filesystem hard links atomically reject existing names.
                match std::fs::hard_link(&self.staging, &result.destination) {
                    Ok(()) => {
                        result.state = "completed".into();
                        result.bytes = bytes;
                        result.content_hash = Some(hash.clone());
                    }
                    Err(error) => {
                        result.state = "failed".into();
                        result.error = Some(format!(
                            "Could not publish download without overwriting: {error}"
                        ));
                    }
                }
            }),
            Err(error) => self.fail(error),
        }
    }
}

impl Drop for DownloadJob {
    fn drop(&mut self) {
        // Only this randomized file is ours. Never remove the user's destination
        // or recursively delete its parent directory.
        if self.parent.canonicalize().ok().as_ref() == Some(&self.parent) {
            let _ = std::fs::remove_file(&self.staging);
        }
    }
}

#[derive(Default)]
pub(super) struct DownloadGate(Mutex<Option<Weak<DownloadJob>>>);

pub(super) struct DownloadAction(pub Arc<DownloadJob>);
impl DownloadAction {
    pub async fn finish(&self) -> DownloadResult {
        let mut receiver = self.0.result.subscribe();
        while !self.0.terminal() {
            let limit = if receiver.borrow().state == "waiting" {
                Duration::from_secs(10)
            } else {
                DOWNLOAD_TIMEOUT
            };
            let deadline =
                tokio::time::Instant::now() + limit.saturating_sub(self.0.started.elapsed());
            if tokio::time::timeout_at(deadline, receiver.changed())
                .await
                .is_err()
            {
                self.0
                    .cancelled
                    .store(true, std::sync::atomic::Ordering::Release);
                self.0.fail("Download timed out".into());
            }
        }
        let result = receiver.borrow().clone();
        result
    }
    pub fn snapshot(&self) -> DownloadResult {
        self.0.result.borrow().clone()
    }
}
impl Drop for DownloadAction {
    fn drop(&mut self) {
        if !self.0.terminal() {
            self.0
                .cancelled
                .store(true, std::sync::atomic::Ordering::Release);
            self.0.fail("Download action was cancelled".into());
        }
    }
}

impl DownloadGate {
    pub fn arm(&self, destination: &Path) -> Result<DownloadAction, String> {
        let mut active = self
            .0
            .lock()
            .map_err(|_| "Browser download gate is unavailable")?;
        if active
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|job| !job.terminal())
        {
            return Err("A download is already active in this tab".into());
        }
        if destination
            .try_exists()
            .map_err(|error| error.to_string())?
        {
            return Err("Download destination already exists".into());
        }
        let parent = destination
            .parent()
            .ok_or("Download requires a destination directory")?
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let destination = parent.join(
            destination
                .file_name()
                .ok_or("Download requires a filename")?,
        );
        let id = uuid::Uuid::new_v4().to_string();
        let staging = parent.join(format!(".nexa-download-{id}.partial"));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
            .map_err(|error| error.to_string())?;
        let (result, _) = watch::channel(DownloadResult {
            id,
            state: "waiting".into(),
            destination,
            bytes: 0,
            content_hash: None,
            error: None,
        });
        let job = Arc::new(DownloadJob {
            result,
            staging,
            parent,
            started: Instant::now(),
            cancelled: Default::default(),
        });
        *active = Some(Arc::downgrade(&job));
        Ok(DownloadAction(job))
    }

    fn claim(&self) -> Option<Arc<DownloadJob>> {
        let job = self.0.lock().ok()?.as_ref()?.upgrade()?;
        let mut claimed = false;
        job.result.send_modify(|result| {
            if result.state == "waiting"
                && job.started.elapsed() < Duration::from_secs(10)
                && !job.cancelled()
            {
                result.state = "receiving".into();
                claimed = true;
            }
        });
        claimed.then_some(job)
    }
    pub fn cancel(&self) {
        if let Some(job) = self
            .0
            .lock()
            .ok()
            .and_then(|active| active.as_ref().and_then(Weak::upgrade))
        {
            job.cancelled
                .store(true, std::sync::atomic::Ordering::Release);
            job.fail("Browser control or tab lifetime ended".into());
        }
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use webview2_com::{
        BytesReceivedChangedEventHandler, CoTaskMemPWSTR, DownloadStartingEventHandler,
        StateChangedEventHandler,
    };
    use windows_core_webview2::Interface;

    struct Operation {
        operation: ICoreWebView2DownloadOperation,
        progress: i64,
        state: i64,
    }
    impl Drop for Operation {
        fn drop(&mut self) {
            unsafe {
                let _ = self.operation.remove_BytesReceivedChanged(self.progress);
                let _ = self.operation.remove_StateChanged(self.state);
            }
        }
    }
    thread_local! { static OPERATIONS: RefCell<HashMap<String, Operation>> = RefCell::default(); }

    pub async fn install(
        webview: &tauri::Webview,
        gate: Arc<DownloadGate>,
        restricted: Arc<AtomicBool>,
    ) -> Result<(), String> {
        use tauri::Manager;
        let app = webview.app_handle().clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        webview.with_webview(move |platform| unsafe {
            let installed = (|| -> windows_core_webview2::Result<()> {
                let core: ICoreWebView2_4 = platform.controller().CoreWebView2()?.cast()?;
                let handler = DownloadStartingEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()); };
                    // Manual browsing retains WebView2's normal download UI.
                    if !restricted.load(Ordering::Acquire) { args.SetCancel(false)?; args.SetHandled(false)?; return Ok(()); }
                    args.SetCancel(true)?;
                    args.SetHandled(true)?;
                    let Some(job) = gate.claim() else { return Ok(()); };
                    let setup = (|| -> windows_core_webview2::Result<()> {
                        let operation = args.DownloadOperation()?;
                        let mut total = 0;
                        operation.TotalBytesToReceive(&mut total)?;
                        if total > MAX_BYTES as i64 { job.fail("Download exceeds 100 MiB".into()); return Ok(()); }
                        let native_path = super::super::webview_host::native_file_path(&job.staging);
                        let path = CoTaskMemPWSTR::from(native_path.as_str());
                        args.SetResultFilePath(*path.as_ref().as_pcwstr())?;
                        let progress_job = job.clone();
                        let mut progress_token = 0;
                        operation.add_BytesReceivedChanged(&BytesReceivedChangedEventHandler::create(Box::new(move |operation, _| {
                            let Some(operation) = operation else { return Ok(()); };
                            let mut bytes = 0;
                            operation.BytesReceived(&mut bytes)?;
                            if bytes > MAX_BYTES as i64 || progress_job.cancelled() {
                                progress_job.fail("Download exceeded its limit or was cancelled".into());
                                operation.Cancel()?;
                            } else { progress_job.result.send_modify(|result| result.bytes = bytes.max(0) as u64); }
                            Ok(())
                        })), &mut progress_token)?;
                        let state_job = job.clone();
                        let mut state_token = 0;
                        if let Err(error) = operation.add_StateChanged(&StateChangedEventHandler::create(Box::new(move |operation, _| {
                            let Some(operation) = operation else { return Ok(()); };
                            let mut state = COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS;
                            operation.State(&mut state)?;
                            if state != COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS {
                                OPERATIONS.with(|operations| { operations.borrow_mut().remove(&state_job.id()); });
                                if state == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
                                    let mut raw_path = windows_core_webview2::PWSTR::null();
                                    operation.ResultFilePath(&mut raw_path)?;
                                    let actual_path = PathBuf::from(CoTaskMemPWSTR::from(raw_path).to_string());
                                    let mut bytes = 0;
                                    operation.BytesReceived(&mut bytes)?;
                                    if actual_path.canonicalize().ok() != state_job.staging.canonicalize().ok() || bytes < 0 || bytes > MAX_BYTES as i64 {
                                        state_job.fail("Native download result path or size did not match its authorized file".into()); return Ok(());
                                    }
                                    state_job.result.send_modify(|result| result.bytes = bytes as u64);
                                    state_job.result.send_modify(|result| { if result.state == "receiving" { result.state = "verifying".into(); } });
                                    let job = state_job.clone();
                                    tauri::async_runtime::spawn_blocking(move || job.complete());
                                } else {
                                    let mut reason = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NONE;
                                    let _ = operation.InterruptReason(&mut reason);
                                    state_job.fail(format!("Native browser download was interrupted: {reason:?}"));
                                }
                            }
                            Ok(())
                        })), &mut state_token) {
                            let _ = operation.remove_BytesReceivedChanged(progress_token);
                            return Err(error);
                        }
                        OPERATIONS.with(|operations| { operations.borrow_mut().insert(job.id(), Operation { operation, progress: progress_token, state: state_token }); });
                        args.SetCancel(false)?;
                        Ok(())
                    })();
                    if let Err(error) = setup { job.fail(format!("Could not start native download: {error}")); }
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        loop {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                            if job.cancelled() || job.terminal() {
                                let id = job.id();
                                let cleanup_job = job.clone();
                                // Use the app dispatcher even if the owning WebView closed.
                                let _ = app.run_on_main_thread(move || {
                                    let operation = OPERATIONS.with(|operations| operations.borrow_mut().remove(&id));
                                    if let Some(operation) = operation { let _ = operation.operation.Cancel(); }
                                    drop(cleanup_job);
                                });
                                if job.cancelled() { job.fail("Download cancelled or timed out".into()); }
                                break;
                            }
                        }
                    });
                    Ok(())
                }));
                let mut token = 0;
                core.add_DownloadStarting(&handler, &mut token)
            })();
            let _ = sender.send(installed.map_err(|error| error.to_string()));
        }).map_err(|error| error.to_string())?;
        tokio::time::timeout(Duration::from_secs(5), receiver)
            .await
            .map_err(|_| "Browser download initialization timed out".to_string())?
            .map_err(|_| "Browser download initialization dropped".to_string())?
    }
}
#[cfg(windows)]
pub(super) use native::install;

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn download_claim_is_single_use_and_publication_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("report.txt");
        let gate = DownloadGate::default();
        let action = gate.arm(&path).unwrap();
        assert!(gate.arm(&root.path().join("second.txt")).is_err());
        let job = gate.claim().unwrap();
        assert!(gate.claim().is_none());
        std::fs::write(&job.staging, "download proof").unwrap();
        job.result.send_modify(|result| result.bytes = 14);
        job.complete();
        let result = action.finish().await;
        assert_eq!(result.state, "completed");
        assert_eq!(result.bytes, 14);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "download proof");
        assert!(result.content_hash.is_some());
        let other = root.path().join("raced.txt");
        let action = gate.arm(&other).unwrap();
        let job = gate.claim().unwrap();
        std::fs::write(&other, "user file").unwrap();
        std::fs::write(&job.staging, "download").unwrap();
        job.result.send_modify(|result| result.bytes = 8);
        job.complete();
        assert_eq!(action.finish().await.state, "failed");
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "user file");
    }
    #[tokio::test]
    async fn cancellation_drops_unclaimed_authorization_and_partial_files() {
        let root = tempfile::tempdir().unwrap();
        let gate = DownloadGate::default();
        let action = gate.arm(&root.path().join("cancelled.txt")).unwrap();
        let staging = action.0.staging.clone();
        gate.cancel();
        assert!(gate.claim().is_none());
        assert_eq!(action.finish().await.state, "failed");
        drop(action);
        assert!(!staging.exists());
    }
}
