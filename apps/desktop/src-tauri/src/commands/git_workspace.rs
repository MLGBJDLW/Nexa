use super::AppState;
use futures::{stream, StreamExt};
use nexa_core::git_workspace::{self, GitWorkspaceStatus};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationGitStatus {
    source_id: String,
    #[serde(flatten)]
    status: GitWorkspaceStatus,
}

async fn sources(
    state: &AppState,
    conversation_id: String,
) -> Result<Vec<(String, PathBuf)>, String> {
    state
        .db_executor
        .read(move |db| {
            db.get_effective_conversation_source_scope(&conversation_id)?
                .into_iter()
                .map(|id| {
                    db.get_source(&id)
                        .map(|source| (id, PathBuf::from(source.root_path)))
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .await
        .map(|result| result.value)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn conversation_git_status_cmd(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<ConversationGitStatus>, String> {
    let roots = sources(&state, conversation_id).await?;
    let results = stream::iter(roots.into_iter().map(|(source_id, root)| async move {
        if !root.is_dir() {
            return Ok(None);
        }
        git_workspace::status(&root)
            .await
            .map(|status| status.map(|status| ConversationGitStatus { source_id, status }))
    }))
    .buffered(4)
    .collect::<Vec<Result<_, String>>>()
    .await;
    results
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map(|results| results.into_iter().flatten().collect())
}

#[tauri::command]
pub async fn conversation_git_diff_cmd(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
    source_id: String,
    path: String,
    staged: bool,
) -> Result<String, String> {
    let root = sources(&state, conversation_id)
        .await?
        .into_iter()
        .find(|(id, _)| id == &source_id)
        .map(|(_, root)| root)
        .ok_or("Git source is no longer in this conversation's scope")?;
    git_workspace::diff(&root, &path, staged).await
}
