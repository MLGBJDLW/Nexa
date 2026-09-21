//! Installed application discovery is a separate authority from workspace paths.
//! A short-lived, conversation-scoped catalog token resolves only the exact
//! executable shown by the launch approval, without accepting command operands.

use std::collections::VecDeque;
#[cfg(any(windows, test))]
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;

use crate::error::CoreError;

const CATALOG_TTL: Duration = Duration::from_secs(300);
const MAX_CATALOG_ENTRIES: usize = 256;

#[derive(Clone, Debug)]
pub(super) struct InstalledApplication {
    pub name: String,
    pub executable: PathBuf,
    pub executable_name: String,
    pub registration: String,
    size: u64,
    modified: SystemTime,
}

impl InstalledApplication {
    #[cfg(any(windows, test))]
    fn from_registered_path(path: &str, registration: &str) -> Option<Self> {
        let path = path.trim();
        let path = path
            .strip_prefix('"')
            .and_then(|path| path.strip_suffix('"'))
            .unwrap_or(path);
        if path.is_empty() || path.contains(['\0', '"', '%']) {
            return None;
        }
        let path = Path::new(path);
        if !path.is_absolute()
            || !path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        {
            return None;
        }
        let executable = path.canonicalize().ok()?;
        if !executable
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        {
            return None;
        }
        let metadata = executable.metadata().ok()?;
        if !metadata.is_file() {
            return None;
        }
        let executable_name = executable.file_name()?.to_str()?.to_string();
        let name = match executable_name.to_ascii_lowercase().as_str() {
            "notepad.exe" => "Notepad 记事本",
            "calc.exe" | "calculator.exe" => "Calculator 计算器",
            "explorer.exe" => "File Explorer 文件资源管理器",
            "excel.exe" => "Microsoft Excel 电子表格",
            "winword.exe" => "Microsoft Word 文字处理",
            "powerpnt.exe" => "Microsoft PowerPoint 演示文稿",
            "outlook.exe" => "Microsoft Outlook",
            "msedge.exe" => "Microsoft Edge",
            "chrome.exe" => "Google Chrome",
            "wechat.exe" | "weixin.exe" => "WeChat 微信",
            _ => executable_name.trim_end_matches(".exe"),
        }
        .to_string();
        Some(Self {
            name,
            executable,
            executable_name,
            registration: registration.to_string(),
            size: metadata.len(),
            modified: metadata.modified().ok()?,
        })
    }

    pub fn verify(&self) -> Result<(), CoreError> {
        let metadata = self.executable.metadata().map_err(|_| stale_catalog())?;
        if !metadata.is_file()
            || metadata.len() != self.size
            || metadata.modified().ok() != Some(self.modified)
            || self.executable.canonicalize().ok().as_ref() != Some(&self.executable)
        {
            return Err(stale_catalog());
        }
        Ok(())
    }
}

fn stale_catalog() -> CoreError {
    CoreError::InvalidInput("Installed application identity changed, or the catalog token expired, was already consumed, or belongs to another conversation. Inspect desktop state and run desktop_automation list_apps again before requesting launch.".into())
}

struct CatalogEntry {
    id: String,
    conversation_id: Option<String>,
    created_at: Instant,
    application: InstalledApplication,
    claimed: bool,
}

fn catalog() -> &'static Mutex<VecDeque<CatalogEntry>> {
    static CATALOG: OnceLock<Mutex<VecDeque<CatalogEntry>>> = OnceLock::new();
    CATALOG.get_or_init(|| Mutex::new(VecDeque::new()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DiscoveredApplication {
    app_id: String,
    name: String,
    executable_name: String,
    executable_path: String,
    registration: String,
    expires_in_seconds: u64,
}

fn remember_applications(
    conversation_id: Option<&str>,
    applications: Vec<InstalledApplication>,
) -> Result<Vec<DiscoveredApplication>, CoreError> {
    let mut entries = catalog()
        .lock()
        .map_err(|_| CoreError::Internal("Installed application catalog is unavailable".into()))?;
    let now = Instant::now();
    entries.retain(|entry| now.duration_since(entry.created_at) <= CATALOG_TTL);
    let mut discovered = Vec::with_capacity(applications.len());
    for application in applications {
        while entries.len() >= MAX_CATALOG_ENTRIES {
            entries.pop_front();
        }
        let id = format!("app-{}", uuid::Uuid::new_v4());
        discovered.push(DiscoveredApplication {
            app_id: id.clone(),
            name: application.name.clone(),
            executable_name: application.executable_name.clone(),
            executable_path: application.executable.to_string_lossy().into_owned(),
            registration: application.registration.clone(),
            expires_in_seconds: CATALOG_TTL.as_secs(),
        });
        entries.push_back(CatalogEntry {
            id,
            conversation_id: conversation_id.map(str::to_owned),
            created_at: now,
            application,
            claimed: false,
        });
    }
    Ok(discovered)
}

pub(super) fn resolve_application(
    conversation_id: Option<&str>,
    app_id: &str,
) -> Result<InstalledApplication, CoreError> {
    let application = {
        let entries = catalog().lock().map_err(|_| {
            CoreError::Internal("Installed application catalog is unavailable".into())
        })?;
        let entry = entries
            .iter()
            .find(|entry| entry.id == app_id)
            .filter(|entry| entry.conversation_id.as_deref() == conversation_id)
            .filter(|entry| entry.created_at.elapsed() <= CATALOG_TTL)
            .filter(|entry| !entry.claimed)
            .ok_or_else(stale_catalog)?;
        entry.application.clone()
    };
    application.verify()?;
    Ok(application)
}

pub(super) fn claim_application(
    conversation_id: Option<&str>,
    app_id: &str,
) -> Result<InstalledApplication, CoreError> {
    let application = resolve_application(conversation_id, app_id)?;
    let mut entries = catalog()
        .lock()
        .map_err(|_| CoreError::Internal("Installed application catalog is unavailable".into()))?;
    let entry = entries
        .iter_mut()
        .find(|entry| entry.id == app_id)
        .filter(|entry| entry.conversation_id.as_deref() == conversation_id)
        .filter(|entry| !entry.claimed && entry.created_at.elapsed() <= CATALOG_TTL)
        .ok_or_else(stale_catalog)?;
    entry.claimed = true;
    Ok(application)
}

pub(super) fn discover_applications(
    conversation_id: Option<&str>,
    query: Option<&str>,
    max_results: usize,
) -> Result<Vec<DiscoveredApplication>, CoreError> {
    if !(1..=50).contains(&max_results) || query.is_some_and(|query| query.chars().count() > 120) {
        return Err(CoreError::InvalidInput(
            "list_apps accepts max_results 1..50 and query up to 120 characters".into(),
        ));
    }
    let query = query.unwrap_or("").trim().to_lowercase();
    let mut applications = platform_applications()?;
    applications.retain(|application| {
        query.is_empty()
            || application.name.to_lowercase().contains(&query)
            || application.executable_name.to_lowercase().contains(&query)
    });
    applications.sort_by_key(|application| {
        (
            application.name.to_lowercase(),
            application.executable.clone(),
        )
    });
    applications.dedup_by(|left, right| {
        left.executable
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.executable.to_string_lossy())
    });
    applications.truncate(max_results);
    remember_applications(conversation_id, applications)
}

#[cfg(windows)]
fn platform_applications() -> Result<Vec<InstalledApplication>, CoreError> {
    let mut applications = Vec::new();
    for (root, root_label) in [
        (windows_registry::CURRENT_USER, "HKCU"),
        (windows_registry::LOCAL_MACHINE, "HKLM"),
    ] {
        for path in [
            r"Software\Microsoft\Windows\CurrentVersion\App Paths",
            r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths",
        ] {
            let Ok(key) = root.open(path) else {
                continue;
            };
            let Ok(names) = key.keys() else {
                continue;
            };
            for name in names.take(512) {
                let Ok(entry) = key.open(&name) else {
                    continue;
                };
                let Ok(executable) = entry.get_string("") else {
                    continue;
                };
                if let Some(application) = InstalledApplication::from_registered_path(
                    &executable,
                    &format!("{root_label}\\{path}\\{name}"),
                ) {
                    applications.push(application);
                }
            }
        }
    }
    // OS registrations, not PATH searching or model-supplied executable paths.
    if let Ok(windows) = windows_registry::LOCAL_MACHINE
        .open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|key| key.get_string("SystemRoot"))
    {
        for relative in [
            r"System32\notepad.exe",
            r"System32\calc.exe",
            "explorer.exe",
        ] {
            if let Some(application) = InstalledApplication::from_registered_path(
                &Path::new(&windows).join(relative).to_string_lossy(),
                "Windows system application",
            ) {
                applications.push(application);
            }
        }
    }
    Ok(applications)
}

#[cfg(not(windows))]
fn platform_applications() -> Result<Vec<InstalledApplication>, CoreError> {
    Err(CoreError::InvalidInput("Installed application discovery currently requires Windows. Source-scoped desktop file actions remain available.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_catalog_tokens_bind_conversation_and_unchanged_executable() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("fixture.exe");
        std::fs::write(&executable, b"fixture").unwrap();
        let application = InstalledApplication::from_registered_path(
            executable.to_str().unwrap(),
            "fixture registration",
        )
        .unwrap();
        let entries = remember_applications(Some("catalog-owner"), vec![application]).unwrap();
        let id = &entries[0].app_id;
        assert!(resolve_application(Some("catalog-owner"), id).is_ok());
        assert!(resolve_application(Some("other-conversation"), id).is_err());
        assert!(resolve_application(None, id).is_err());
        assert!(resolve_application(Some("catalog-owner"), executable.to_str().unwrap()).is_err());
        std::fs::write(&executable, b"changed executable").unwrap();
        assert!(resolve_application(Some("catalog-owner"), id).is_err());
    }

    #[test]
    fn registrations_never_parse_commands_or_relative_programs() {
        for path in [
            "notepad.exe",
            "https://example.com/app.exe",
            "cmd.exe /c command",
            "\"C:\\app.exe\" --run",
            "%TEMP%\\app.exe",
        ] {
            assert!(InstalledApplication::from_registered_path(path, "fixture").is_none());
        }
    }

    #[test]
    fn expired_catalog_tokens_require_discovery_again() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("fixture.exe");
        std::fs::write(&executable, b"fixture").unwrap();
        let application = InstalledApplication::from_registered_path(
            executable.to_str().unwrap(),
            "fixture registration",
        )
        .unwrap();
        let discovered = remember_applications(Some("expired-catalog"), vec![application]).unwrap();
        let id = &discovered[0].app_id;
        catalog()
            .lock()
            .unwrap()
            .iter_mut()
            .find(|entry| &entry.id == id)
            .unwrap()
            .created_at = Instant::now() - CATALOG_TTL - Duration::from_secs(1);
        assert!(resolve_application(Some("expired-catalog"), id).is_err());
    }

    #[test]
    fn preview_does_not_consume_but_launch_claim_is_single_use() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("fixture.exe");
        std::fs::write(&executable, b"fixture").unwrap();
        let application =
            InstalledApplication::from_registered_path(executable.to_str().unwrap(), "fixture")
                .unwrap();
        let discovered = remember_applications(Some("single-use"), vec![application]).unwrap();
        let id = &discovered[0].app_id;
        assert!(resolve_application(Some("single-use"), id).is_ok());
        assert!(resolve_application(Some("single-use"), id).is_ok());
        assert!(claim_application(Some("single-use"), id).is_ok());
        assert!(claim_application(Some("single-use"), id).is_err());
        assert!(resolve_application(Some("single-use"), id).is_err());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "reads installed Windows application registrations"]
    fn windows_catalog_discovers_notepad_without_workspace_sources() {
        let applications =
            discover_applications(Some("catalog-native-smoke"), Some("记事本"), 10).unwrap();
        let application = applications
            .iter()
            .find(|application| {
                application
                    .executable_name
                    .eq_ignore_ascii_case("notepad.exe")
            })
            .expect("Windows Notepad should be discoverable by its localized app name");
        let resolved =
            resolve_application(Some("catalog-native-smoke"), &application.app_id).unwrap();
        assert!(resolved.executable.is_file());
        assert!(resolved.executable.is_absolute());
    }
}
