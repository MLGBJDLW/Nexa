use super::AppState;
use nexa_core::code_review::{self, FindingInput};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewRequest {
    Get,
    List,
    Start {
        mode: String,
        base: String,
    },
    Select {
        id: String,
    },
    Add {
        review_id: String,
        finding: FindingInput,
    },
    Disposition {
        review_id: String,
        revision: String,
        finding_id: String,
        status: String,
    },
    Feedback {
        review_id: String,
        revision: String,
        ids: Vec<String>,
    },
    AttachPr {
        review_id: String,
        url: String,
    },
}
#[tauri::command]
pub async fn code_review_cmd(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
    request: ReviewRequest,
) -> Result<Value, String> {
    let db = &state.db;
    let result = async {
        match request {
            ReviewRequest::Get => Ok(code_review::get(db, &conversation_id)
                .await?
                .map(|review| review.view(true))
                .unwrap_or(Value::Null)),
            ReviewRequest::List => Ok(serde_json::to_value(code_review::list(
                db,
                &conversation_id,
            )?)?),
            ReviewRequest::Start { mode, base } => {
                Ok(code_review::start(db, &conversation_id, &mode, &base)
                    .await?
                    .view(true))
            }
            ReviewRequest::Select { id } => Ok(code_review::select(db, &conversation_id, &id)
                .await?
                .view(true)),
            ReviewRequest::Add { review_id, finding } => {
                Ok(code_review::add(db, &conversation_id, &review_id, finding)
                    .await?
                    .view(true))
            }
            ReviewRequest::Disposition {
                review_id,
                revision,
                finding_id,
                status,
            } => Ok(code_review::disposition(
                db,
                &conversation_id,
                &review_id,
                &revision,
                &finding_id,
                &status,
            )
            .await?
            .view(true)),
            ReviewRequest::Feedback {
                review_id,
                revision,
                ids,
            } => code_review::feedback(db, &conversation_id, &review_id, &revision, &ids).await,
            ReviewRequest::AttachPr { review_id, url } => {
                Ok(
                    code_review::attach_pr(db, &conversation_id, &review_id, &url)
                        .await?
                        .view(true),
                )
            }
        }
    }
    .await;
    result.map_err(|error: nexa_core::error::CoreError| error.to_string())
}
