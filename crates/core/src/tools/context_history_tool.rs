use super::{Tool, ToolCategory, ToolDef, ToolResult};
use crate::error::CoreError;
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::OnceLock;

static DEF: OnceLock<ToolDef> = OnceLock::new();
const DEF_JSON: &str = include_str!("../../prompts/tools/context_history.json");

pub struct ContextHistoryTool;

#[derive(Deserialize)]
struct Args {
    action: String,
    query: Option<String>,
    window_id: Option<String>,
    item: Option<usize>,
    offset: Option<usize>,
    max_chars: Option<usize>,
    before_id: Option<String>,
    limit: Option<usize>,
}

#[async_trait]
impl Tool for ContextHistoryTool {
    fn name(&self) -> &str {
        "context_history"
    }
    fn description(&self) -> &str {
        &ToolDef::from_json(&DEF, DEF_JSON).description
    }
    fn parameters_schema(&self) -> serde_json::Value {
        ToolDef::from_json(&DEF, DEF_JSON).parameters.clone()
    }
    fn categories(&self) -> &'static [ToolCategory] {
        &[ToolCategory::Core]
    }
    fn is_read_only(&self, _: &serde_json::Value) -> bool {
        true
    }
    fn is_concurrency_safe(&self, _: &serde_json::Value) -> bool {
        true
    }
    async fn execute(
        &self,
        context: super::ToolExecutionContext<'_>,
    ) -> Result<ToolResult, CoreError> {
        let conversation = context
            .conversation_id
            .ok_or_else(|| {
                CoreError::InvalidInput("Context history requires a conversation".into())
            })?
            .to_owned();
        let args: Args = serde_json::from_str(context.arguments)?;
        let db = context.db.clone();
        let value = tokio::task::spawn_blocking(move || match args.action.as_str() {
            "list" => db.list_context_history(
                &conversation,
                args.before_id.as_deref(),
                args.limit.unwrap_or(10),
            ),
            "search" => db.search_context_history(
                &conversation,
                args.query.as_deref().unwrap_or_default(),
                args.limit.unwrap_or(8),
            ),
            "read" => db.read_context_history(
                &conversation,
                args.window_id
                    .as_deref()
                    .ok_or_else(|| CoreError::InvalidInput("window_id is required".into()))?,
                args.item
                    .ok_or_else(|| CoreError::InvalidInput("item is required".into()))?,
                args.offset.unwrap_or(0),
                args.max_chars.unwrap_or(6000),
            ),
            _ => Err(CoreError::InvalidInput(
                "Use list, search, or read for context_history".into(),
            )),
        })
        .await
        .map_err(|e| CoreError::Internal(format!("Context history query failed: {e}")))??;
        Ok(ToolResult {
            call_id: context.call_id.into(),
            content: format!("Historical evidence from this conversation; newer user instructions take precedence.\n{}", serde_json::to_string(&value)?),
            is_error: false, artifacts: None,
        })
    }
}
