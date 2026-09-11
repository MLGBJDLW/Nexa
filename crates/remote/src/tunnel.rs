//! Optional outbound Quick Tunnel. The host explicitly enables it; no account credentials are used.
use futures::StreamExt;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

pub struct ManagedTunnel {
    pub url: String,
    alternatives: Arc<Mutex<Vec<String>>>,
    stop: CancellationToken,
    running: Arc<AtomicBool>,
}
impl ManagedTunnel {
    pub fn urls(&self) -> Vec<String> {
        let mut urls = vec![self.url.clone()];
        for url in self
            .alternatives
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            if !urls.contains(url) {
                urls.push(url.clone());
            }
        }
        urls
    }
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }
    pub async fn stop(&self) -> bool {
        self.stop.cancel();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while self.is_running() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        !self.is_running()
    }
}
impl Drop for ManagedTunnel {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
struct TemporaryDownload(PathBuf);
impl Drop for TemporaryDownload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn helper(directory: &Path) -> Result<PathBuf, String> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    let asset_name = "cloudflared-windows-amd64.exe";
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    let asset_name = "cloudflared-linux-amd64";
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    let asset_name = "cloudflared-linux-arm64";
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64")
    )))]
    let asset_name = "unsupported";
    if asset_name == "unsupported" {
        return Err("Use a fixed HTTPS tunnel address on this platform".into());
    }
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .user_agent("Nexa-Remote")
        .http1_only()
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    // Immutable official release metadata ships with Nexa. Cached helpers can
    // start offline, without a GitHub API request or anonymous rate-limit budget.
    let release: serde_json::Value = serde_json::from_str(include_str!("cloudflared-release.json"))
        .map_err(|_| "Invalid bundled tunnel release metadata")?;
    let asset = release["assets"]
        .as_array()
        .and_then(|assets| {
            assets
                .iter()
                .find(|asset| asset["name"].as_str() == Some(asset_name))
        })
        .ok_or("Official tunnel helper is unavailable")?;
    let digest = asset["digest"]
        .as_str()
        .and_then(|value| value.strip_prefix("sha256:"))
        .filter(|value| value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or("Official release has no SHA-256 digest; refusing an unverified helper")?;
    let url = asset["browser_download_url"]
        .as_str()
        .filter(|value| {
            value.starts_with("https://github.com/cloudflare/cloudflared/releases/download/")
        })
        .ok_or("Unexpected helper download address")?;
    let binary = directory.join(format!("{}-{asset_name}", &digest[..16]));
    if binary.exists() {
        let bytes = tokio::fs::read(&binary).await.map_err(|e| e.to_string())?;
        if format!("{:x}", Sha256::digest(&bytes)) == digest {
            return Ok(binary);
        }
        return Err("Cached tunnel helper failed verification".into());
    }
    if let Some(parent) = directory.parent() {
        let legacy = parent.join(format!("{}-{asset_name}", &digest[..16]));
        if let Ok(bytes) = tokio::fs::read(&legacy).await {
            if format!("{:x}", Sha256::digest(&bytes)) == digest {
                tokio::fs::copy(&legacy, &binary)
                    .await
                    .map_err(|e| e.to_string())?;
                return Ok(binary);
            }
        }
    }
    let temporary = directory.join(format!("{}.download", uuid::Uuid::new_v4()));
    let _temporary_guard = TemporaryDownload(temporary.clone());
    let result = async {
        let mut response = client
            .get(url)
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .bytes_stream();
        let mut file = tokio::fs::File::create(&temporary)
            .await
            .map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut total = 0;
        while let Some(chunk) = response.next().await {
            let chunk = chunk.map_err(|e| {
                format!("Tunnel helper download interrupted after {total} bytes: {e}")
            })?;
            total += chunk.len();
            if total > 100 * 1024 * 1024 {
                return Err("Tunnel helper download exceeds the expected size".into());
            }
            hash.update(&chunk);
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        }
        file.flush().await.map_err(|e| e.to_string())?;
        drop(file);
        if format!("{:x}", hash.finalize()) != digest {
            return Err("Tunnel helper SHA-256 verification failed".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o700))
                .await
                .map_err(|e| e.to_string())?;
        }
        tokio::fs::rename(&temporary, &binary)
            .await
            .map_err(|e| e.to_string())?;
        Ok(binary)
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}
pub async fn start(directory: &Path, loopback: &str) -> Result<ManagedTunnel, String> {
    let parsed = url::Url::parse(loopback).map_err(|_| "Invalid tunnel target")?;
    if parsed.scheme() != "http" || parsed.host_str() != Some("127.0.0.1") {
        return Err("Managed tunnels only forward Nexa's loopback listener".into());
    }
    let binary = helper(directory).await?;
    let config = directory.join("quick-tunnel-empty.yml");
    tokio::fs::write(&config, "{}\n")
        .await
        .map_err(|e| e.to_string())?;
    let mut command = tokio::process::Command::new(binary);
    command
        .args(["tunnel", "--no-autoupdate", "--config"])
        .arg(config)
        .args(["--url", loopback, "--protocol", "http2"])
        .env_remove("TUNNEL_TOKEN")
        .env_remove("TUNNEL_ORIGIN_CERT")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("TUNNEL_")
        {
            command.env_remove(key);
        }
    }
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Unable to start tunnel helper: {e}"))?;
    #[cfg(windows)]
    let job = WindowsJob::attach(&child)?;
    let stderr = child.stderr.take().ok_or("Tunnel output is unavailable")?;
    let (ready, receive) = tokio::sync::oneshot::channel();
    let stop = CancellationToken::new();
    let cancelled = stop.clone();
    let running = Arc::new(AtomicBool::new(true));
    let alive = running.clone();
    let diagnostic = Arc::new(Mutex::new(String::new()));
    let last_diagnostic = diagnostic.clone();
    tokio::spawn(async move {
        #[cfg(windows)]
        let _job = job;
        let mut reader = BufReader::new(stderr);
        let mut line = Vec::new();
        let mut ready = Some(ready);
        let mut readiness = TunnelReadiness::default();
        loop {
            line.clear();
            let read = tokio::select! {_=cancelled.cancelled()=>break,result=reader.read_until(b'\n',&mut line)=>result};
            if !matches!(read,Ok(size) if size>0) || line.len() > 32 * 1024 {
                break;
            }
            let message = String::from_utf8_lossy(&line);
            if message.contains(" ERR ") || message.contains("failed") {
                *last_diagnostic.lock().unwrap_or_else(|e| e.into_inner()) =
                    message.trim().chars().take(768).collect();
            }
            if let Some(url) = readiness.observe(&message) {
                if let Some(sender) = ready.take() {
                    let _ = sender.send(url);
                }
            }
        }
        let _ = child.kill().await;
        let _ = child.wait().await;
        alive.store(false, Ordering::Release);
    });
    let mut tunnel = ManagedTunnel {
        url: String::new(),
        alternatives: Arc::default(),
        stop,
        running,
    };
    tunnel.url = tokio::time::timeout(Duration::from_secs(60), receive)
        .await
        .map_err(|_| {
            let detail = diagnostic.lock().unwrap_or_else(|e| e.into_inner());
            format!("The public tunnel could not connect to its edge within 60 seconds. {detail}")
        })?
        .map_err(|_| {
            let detail = diagnostic.lock().unwrap_or_else(|e| e.into_inner());
            format!("The tunnel helper stopped before becoming ready. {detail}")
        })?;
    Ok(tunnel)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicTunnelProvider {
    #[default]
    Auto,
    LocalhostRun,
    Pinggy,
    Cloudflare,
}

impl PublicTunnelProvider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::LocalhostRun => "localhost.run",
            Self::Pinggy => "Pinggy",
            Self::Cloudflare => "Cloudflare",
        }
    }
}

fn ssh_program(name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("System32")
            .join("OpenSSH")
            .join(format!("{name}.exe"))
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/usr/bin").join(name)
    }
}

fn ssh_url(provider: PublicTunnelProvider, line: &str) -> Option<String> {
    if provider == PublicTunnelProvider::LocalhostRun {
        if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
            if event["event"] == "tcpip-forward"
                && event["status"] == "success"
                && event["tls_termination"] == true
            {
                return event["address"]
                    .as_str()
                    .and_then(|host| ssh_url(provider, &format!("https://{host}")));
            }
            return None;
        }
    }
    line.split_whitespace().find_map(|word| {
        let url = url::Url::parse(word.trim_matches(['|', '"', ',', '\''])).ok()?;
        let host = url.host_str()?;
        let suffixes: &[&str] = match provider {
            PublicTunnelProvider::LocalhostRun => &[".lhr.life", ".localhost.run"],
            PublicTunnelProvider::Pinggy => &[
                ".free.pinggy.net",
                ".pinggy.link",
                ".pinggy.online",
                ".run.pinggy-free.link",
            ],
            _ => return None,
        };
        (url.scheme() == "https"
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
            && suffixes.iter().any(|suffix| host.ends_with(suffix)))
        .then(|| url.origin().ascii_serialization())
    })
}

pub async fn start_ssh(
    directory: &Path,
    loopback: &str,
    provider: PublicTunnelProvider,
) -> Result<ManagedTunnel, String> {
    let url = url::Url::parse(loopback).map_err(|_| "Invalid tunnel target")?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.path() != "/"
        || url.query().is_some()
        || !url.username().is_empty()
    {
        return Err("Managed tunnels only forward Nexa's loopback listener".into());
    }
    let port = url.port().ok_or("Missing loopback port")?;
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|e| e.to_string())?;
    let identity = directory.join("anonymous-tunnel-key");
    // This identity is generated exclusively for anonymous tunnel authentication;
    // SSH never reads or offers the user's personal keys or agent identities.
    if provider == PublicTunnelProvider::Pinggy && !identity.exists() {
        let mut keygen = tokio::process::Command::new(ssh_program("ssh-keygen"));
        keygen
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&identity)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        keygen.creation_flags(0x08000000);
        let result = keygen
            .output()
            .await
            .map_err(|_| "OpenSSH is unavailable. Select another public connection provider")?;
        if !result.status.success() {
            return Err("Unable to prepare the anonymous tunnel identity".into());
        }
    }
    let known_hosts = directory.join("known_hosts");
    let mut command = tokio::process::Command::new(ssh_program("ssh"));
    command
        .args([
            "-T",
            "-F",
            "none",
            "-o",
            "BatchMode=yes",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "IdentityAgent=none",
            "-o",
            "PasswordAuthentication=no",
            "-o",
            "KbdInteractiveAuthentication=no",
            "-o",
            "ForwardAgent=no",
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            "ConnectTimeout=8",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=2",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
        ])
        .arg(format!(
            "UserKnownHostsFile=\"{}\"",
            known_hosts
                .to_string_lossy()
                .replace('\\', "/")
                .replace('%', "%%")
                .replace('"', "\\\"")
        ));
    match provider {
        PublicTunnelProvider::LocalhostRun => {
            command
                .args(["-o", "IdentityFile=none", "-R"])
                .arg(format!("80:127.0.0.1:{port}"))
                .args(["nokey@localhost.run", "--", "--output", "json"]);
        }
        PublicTunnelProvider::Pinggy => {
            command
                .arg("-i")
                .arg(&identity)
                .args(["-p", "443", "-R"])
                .arg(format!("0:127.0.0.1:{port}"))
                .args(["a@free.pinggy.io", "x:passpreflight"]);
        }
        _ => return Err("Choose a supported SSH tunnel provider".into()),
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|_| "OpenSSH is unavailable. Select another public connection provider")?;
    #[cfg(windows)]
    let job = WindowsJob::attach(&child)?;
    let stdout = child.stdout.take().ok_or("Tunnel output unavailable")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Tunnel diagnostics unavailable")?;
    let stop = CancellationToken::new();
    let cancelled = stop.clone();
    let running = Arc::new(AtomicBool::new(true));
    let alive = running.clone();
    let alternatives = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed = alternatives.clone();
    let diagnostics = Arc::new(Mutex::new(String::new()));
    let detail = diagnostics.clone();
    let (ready, receive) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        #[cfg(windows)]
        let _job = job;
        let mut ready = Some(ready);
        let mut output = BufReader::new(stdout).lines();
        let mut errors = BufReader::new(stderr).lines();
        loop {
            let line = tokio::select! {
                _ = cancelled.cancelled() => break,
                _ = child.wait() => break,
                line = output.next_line() => line,
                line = errors.next_line() => line,
            };
            let Ok(Some(line)) = line else {
                break;
            };
            if let Some(url) = ssh_url(provider, &line) {
                let mut urls = observed.lock().unwrap_or_else(|e| e.into_inner());
                if urls.len() < 4 && !urls.contains(&url) {
                    urls.push(url.clone());
                }
                if let Some(sender) = ready.take() {
                    let _ = sender.send(url);
                }
            } else if !line.trim().is_empty() && !line.starts_with('{') {
                *detail.lock().unwrap_or_else(|e| e.into_inner()) =
                    line.chars().filter(|c| !c.is_control()).take(300).collect();
            }
        }
        let _ = child.kill().await;
        let _ = child.wait().await;
        alive.store(false, Ordering::Release);
    });
    let mut tunnel = ManagedTunnel {
        url: String::new(),
        alternatives,
        stop,
        running,
    };
    tunnel.url = tokio::time::timeout(Duration::from_secs(20), receive)
        .await
        .map_err(|_| format!("{} connection timed out", provider.name()))?
        .map_err(|_| {
            format!(
                "{} could not connect: {}",
                provider.name(),
                diagnostics.lock().unwrap_or_else(|e| e.into_inner())
            )
        })?;
    Ok(tunnel)
}
#[derive(Default)]
struct TunnelReadiness {
    url: Option<String>,
    registered: bool,
}
impl TunnelReadiness {
    fn observe(&mut self, line: &str) -> Option<String> {
        if let Some(url) = quick_url(line) {
            self.url = Some(url);
        }
        // The helper prints its allocated URL before it establishes an edge connection.
        self.registered |= line.contains("Registered tunnel connection");
        if self.registered {
            self.url.clone()
        } else {
            None
        }
    }
}
fn quick_url(line: &str) -> Option<String> {
    line.split_whitespace().find_map(|word| {
        let parsed = url::Url::parse(word.trim_matches(['|', '"', ','])).ok()?;
        (parsed.scheme() == "https"
            && parsed.host_str()?.ends_with(".trycloudflare.com")
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.path() == "/"
            && parsed.query().is_none())
        .then(|| parsed.origin().ascii_serialization())
    })
}
#[cfg(windows)]
struct WindowsJob(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for WindowsJob {}
#[cfg(windows)]
impl WindowsJob {
    fn attach(child: &tokio::process::Child) -> Result<Self, String> {
        use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
        let process = child.raw_handle().ok_or("Tunnel process is unavailable")?;
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err("Unable to create the tunnel process lifetime guard".into());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&info) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, process as _) == 0
            {
                CloseHandle(handle);
                return Err("Unable to bind the tunnel lifetime to Nexa".into());
            }
            Ok(Self(handle))
        }
    }
}
#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "explicit anonymous SSH helper connectivity test"]
    async fn ssh_helper_smoke() {
        let directory = tempfile::tempdir().unwrap();
        let tunnel = start_ssh(
            directory.path(),
            "http://127.0.0.1:59988",
            PublicTunnelProvider::LocalhostRun,
        )
        .await
        .unwrap();
        assert!(tunnel.is_running());
        println!("Managed SSH helper is ready");
        assert!(tunnel.stop().await);
    }
    #[test]
    fn ssh_tunnel_urls_are_scoped_to_the_selected_provider() {
        assert_eq!(ssh_url(PublicTunnelProvider::LocalhostRun, r#"{"event":"tcpip-forward","status":"success","tls_termination":true,"address":"example.lhr.life"}"#).as_deref(), Some("https://example.lhr.life"));
        assert!(ssh_url(
            PublicTunnelProvider::Pinggy,
            "https://test.run.pinggy-free.link"
        )
        .is_some());
        assert!(ssh_url(PublicTunnelProvider::Pinggy, "https://test.free.pinggy.net").is_some());
        for url in [
            "https://x.lhr.life.evil.test",
            "http://x.lhr.life",
            "https://key@x.lhr.life",
            "https://x.lhr.life/path",
            "https://localhost.run/docs/",
            "https://x.lhr.life#key",
            "https://x.run.pinggy-free.link",
        ] {
            assert!(
                ssh_url(PublicTunnelProvider::LocalhostRun, url).is_none(),
                "unexpected tunnel URL: {url}"
            );
        }
    }
    #[test]
    fn allocated_url_is_not_ready_until_the_edge_registers() {
        let mut state = TunnelReadiness::default();
        assert_eq!(state.observe("| https://example.trycloudflare.com |"), None);
        assert_eq!(state.observe("Registering tunnel connection"), None);
        assert_eq!(
            state.observe("INF Registered tunnel connection"),
            Some("https://example.trycloudflare.com".into())
        );
        let mut reverse = TunnelReadiness::default();
        assert_eq!(reverse.observe("INF Registered tunnel connection"), None);
        assert!(reverse
            .observe("| https://example.trycloudflare.com |")
            .is_some());
    }
    #[test]
    fn accepts_only_official_quick_tunnel_origins() {
        assert_eq!(
            quick_url("| https://example.trycloudflare.com |"),
            Some("https://example.trycloudflare.com".into())
        );
        for line in [
            "https://trycloudflare.com.evil.test",
            "http://x.trycloudflare.com",
            "https://key@x.trycloudflare.com",
            "https://x.trycloudflare.com/a",
        ] {
            assert_eq!(quick_url(line), None);
        }
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn closing_process_guard_terminates_its_child() {
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tunnel::tests::job_child_entry", "--ignored"])
            .env("NEXA_REMOTE_JOB_CHILD", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let job = WindowsJob::attach(&child).unwrap();
        assert!(child.try_wait().unwrap().is_none());
        drop(job);
        tokio::time::timeout(Duration::from_secs(3), child.wait())
            .await
            .unwrap()
            .unwrap();
    }
    #[test]
    #[ignore = "entry point for the process lifetime test"]
    fn job_child_entry() {
        if std::env::var("NEXA_REMOTE_JOB_CHILD").as_deref() == Ok("1") {
            std::thread::sleep(Duration::from_secs(30));
        }
    }
}
