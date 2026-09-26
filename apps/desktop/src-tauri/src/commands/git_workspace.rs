use super::{terminal::TerminalState, AppState};
use nexa_core::git_workspace::{self, GitWorkspaceSnapshot};
use std::path::PathBuf;

async fn sources(
    state: &AppState,
    terminal: &TerminalState,
    conversation_id: String,
) -> Result<Vec<(String, PathBuf)>, String> {
    let session = terminal
        .active_session(&conversation_id)?
        .filter(|session| session.conversation_id.as_deref() == Some(&conversation_id));
    let mut roots = state
        .db_executor
        .read(move |db| git_workspace::conversation_sources(db, &conversation_id))
        .await
        .map(|result| result.value)
        .map_err(|error| error.to_string())?;
    // Only a user-linked live session can contribute a terminal scope. Never
    // infer a path from model text or the desktop application's own cwd.
    if let Some(session) = session {
        roots.insert(
            0,
            (
                format!("terminal:{}", session.id),
                PathBuf::from(session.cwd),
            ),
        );
    }
    Ok(roots)
}

#[tauri::command]
pub async fn conversation_git_status_cmd(
    state: tauri::State<'_, AppState>,
    terminal: tauri::State<'_, TerminalState>,
    conversation_id: String,
) -> Result<GitWorkspaceSnapshot, String> {
    Ok(git_workspace::snapshot(sources(&state, &terminal, conversation_id).await?).await)
}

#[tauri::command]
pub async fn conversation_git_diff_cmd(
    state: tauri::State<'_, AppState>,
    terminal: tauri::State<'_, TerminalState>,
    conversation_id: String,
    source_id: String,
    path: String,
    staged: bool,
) -> Result<String, String> {
    let root = sources(&state, &terminal, conversation_id)
        .await?
        .into_iter()
        .find(|(id, _)| id == &source_id)
        .map(|(_, root)| root)
        .ok_or("Git source is no longer in this conversation's scope")?;
    git_workspace::diff(&root, &path, staged).await
}
