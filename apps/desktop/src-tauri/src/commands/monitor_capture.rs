//! Explicit source picking for screen sharing in desktop WebViews.

#[cfg(windows)]
use nexa_core::tools::computer_use_tool::monitors as monitor_sources;
use nexa_core::tools::computer_use_tool::UserShareMonitor;

#[tauri::command]
pub async fn list_desktop_monitors_cmd() -> Result<Vec<UserShareMonitor>, String> {
    #[cfg(windows)]
    return tokio::task::spawn_blocking(monitor_sources::monitors)
        .await
        .map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    Ok(Vec::new())
}

#[tauri::command]
pub async fn list_desktop_windows_cmd() -> Result<serde_json::Value, String> {
    let windows =
        tokio::task::spawn_blocking(nexa_core::tools::computer_use_tool::list_user_share_windows)
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({"supported": cfg!(windows), "windows": windows}))
}

#[tauri::command]
pub async fn capture_desktop_window_cmd(window_id: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        nexa_core::tools::computer_use_tool::capture_user_share_window(&window_id)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn capture_desktop_monitor_cmd(monitor_id: String) -> Result<String, String> {
    #[cfg(windows)]
    return tokio::task::spawn_blocking(move || monitor_sources::capture_monitor(&monitor_id))
        .await
        .map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    {
        let _ = monitor_id;
        Err("Use the system screen picker on this platform".into())
    }
}
