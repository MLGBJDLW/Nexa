#[tauri::command]
pub fn begin_desktop_share_cmd(conversation_id: String, source: String) -> Result<String, String> {
    nexa_core::shared_desktop::store().begin(&conversation_id, &source)
}
#[tauri::command]
pub async fn update_desktop_share_cmd(
    conversation_id: String,
    lease: String,
    sequence: u64,
    base64: String,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        nexa_core::shared_desktop::store().update(&conversation_id, &lease, sequence, base64)
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub fn end_desktop_share_cmd(conversation_id: String, lease: String) {
    nexa_core::shared_desktop::store().end(&conversation_id, &lease);
}
