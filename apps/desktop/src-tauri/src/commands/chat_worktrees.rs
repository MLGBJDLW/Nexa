use super::AppState;
use nexa_core::chat_worktrees::{self, ChatWorktree};

#[tauri::command]
pub async fn get_chat_worktree_cmd(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<Option<ChatWorktree>, String> {
    chat_worktrees::inspect(&state.db, &conversation_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn change_chat_worktree_cmd(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
    action: String,
    start_ref: Option<String>,
) -> Result<Option<ChatWorktree>, String> {
    let db = state.db.clone();
    // The host owns completion even when the renderer closes the panel.
    tokio::spawn(async move {
        match action.as_str() {
            "create" => chat_worktrees::create(
                &db,
                &conversation_id,
                start_ref.as_deref().unwrap_or("HEAD"),
            )
            .await
            .map(Some),
            "archive" => chat_worktrees::archive(&db, &conversation_id)
                .await
                .map(Some),
            "restore" => chat_worktrees::restore(&db, &conversation_id)
                .await
                .map(Some),
            "recover" => chat_worktrees::recover(&db, &conversation_id).await,
            _ => Err(nexa_core::error::CoreError::InvalidInput(
                "Unknown worktree action".into(),
            )),
        }
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
