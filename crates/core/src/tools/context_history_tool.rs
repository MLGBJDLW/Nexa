use super::{Tool, ToolCategory, ToolDef, ToolRegistry, ToolResult};
use crate::db::Database;
use crate::error::CoreError;
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::OnceLock;

static DEF: OnceLock<ToolDef> = OnceLock::new();
const DEF_JSON: &str = include_str!("../../prompts/tools/context_history.json");

mod code_mode;
pub(crate) mod live;

#[derive(Default)]
pub struct ContextHistoryTool {
    code_mode: bool,
}

impl ContextHistoryTool {
    pub fn new(code_mode: bool) -> Self {
        Self { code_mode }
    }

    /// Enable the action only on a registry which already grants history access.
    pub fn enable_code_mode(registry: &mut ToolRegistry) {
        if registry.contains("context_history") {
            *registry = registry.without_names(&["context_history"]);
            registry.register(Box::new(Self::new(true)));
        }
    }
}

#[derive(Deserialize)]
struct Args {
    action: String,
    query: Option<String>,
    code: Option<String>,
    window_id: Option<String>,
    item: Option<usize>,
    #[serde(flatten)]
    read: ReadResult,
    before_id: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct ReadResult {
    turn_id: Option<String>,
    tool_call_id: Option<String>,
    result_id: Option<String>,
    expected_digest: Option<String>,
    offset: Option<usize>,
    max_chars: Option<usize>,
}

impl ReadResult {
    fn page(&self, db: &Database, conversation: &str) -> Result<serde_json::Value, CoreError> {
        let page = db.read_context_tool_result(
            conversation,
            self.turn_id
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| CoreError::InvalidInput("turn_id is required".into()))?,
            self.tool_call_id
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| CoreError::InvalidInput("tool_call_id is required".into()))?,
            self.result_id.as_deref(),
            self.offset.unwrap_or(0),
            self.max_chars.unwrap_or(6000),
        )?;
        if self
            .expected_digest
            .as_deref()
            .is_some_and(|d| page["contentDigest"].as_str() != Some(d))
        {
            return Err(CoreError::InvalidInput(
                "The persisted tool result changed, possibly after a privacy update. Restart at offset 0 without expected_digest and use the new contentDigest for later pages.".into()
            ));
        }
        Ok(page)
    }
}

#[async_trait]
impl Tool for ContextHistoryTool {
    fn name(&self) -> &str {
        "context_history"
    }
    fn description(&self) -> &str {
        let description = &ToolDef::from_json(&DEF, DEF_JSON).description;
        if self.code_mode {
            static ENABLED: OnceLock<String> = OnceLock::new();
            return ENABLED.get_or_init(|| format!("{description} Use action=code_mode to chain live tools, filter results locally and return one answer."));
        }
        description
    }
    fn parameters_schema(&self) -> serde_json::Value {
        let mut schema = ToolDef::from_json(&DEF, DEF_JSON).parameters.clone();
        if self.code_mode {
            schema["properties"]["action"]["enum"]
                .as_array_mut()
                .unwrap()
                .push("code_mode".into());
            schema["properties"]["code"] = serde_json::json!({
                "type": "string",
                "maxLength": code_mode::MAX_SOURCE_BYTES,
                "description": "Local Luau. list_tools() gives callable names; tool_schema(name) gives arguments. call_tool(name,{key=value}) returns live text. call_tools({{name=...,arguments={...}},...}) batches independent reads in input order. Chain calls, filter their results and return a short string, number, boolean or nil. decode_json(text) parses JSON. For past results, list_results(turn_id?,offset?) and read_result(turnId,toolCallId,resultId?,offset?,expectedDigest?) return pages with nextTurnId/nextOffset or contentDigest."
            });
        }
        schema
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
        if args.action == "code_mode" {
            if !self.code_mode {
                return Err(CoreError::InvalidInput(
                    "Code Mode is disabled in settings.".into(),
                ));
            }
            let code = args
                .code
                .ok_or_else(|| CoreError::InvalidInput("code is required".into()))?;
            let value =
                code_mode::run(context.db.clone(), conversation, code, context.cancel_token)
                    .await?;
            return Ok(ToolResult {
                call_id: context.call_id.into(),
                content: serde_json::to_string(&value)?,
                is_error: false,
                artifacts: None,
            });
        }
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
                args.read.offset.unwrap_or(0),
                args.read.max_chars.unwrap_or(6000),
            ),
            "read_notes" => db.read_context_history_notes(
                &conversation,
                args.window_id
                    .as_deref()
                    .ok_or_else(|| CoreError::InvalidInput("window_id is required".into()))?,
                args.read.offset.unwrap_or(0),
                args.read.max_chars.unwrap_or(6000),
            ),
            "read_tool_result" => args.read.page(&db, &conversation),
            _ => Err(CoreError::InvalidInput(
                "Use list, search, read, read_notes, or read_tool_result for context_history"
                    .into(),
            )),
        })
        .await
        .map_err(|e| CoreError::Internal(format!("Context history query failed: {e}")))??;
        history_result(context.call_id, &value)
    }
}

fn history_result(call_id: &str, value: &serde_json::Value) -> Result<ToolResult, CoreError> {
    Ok(ToolResult {
            call_id: call_id.into(),
            content: format!("Historical evidence from this conversation; newer user instructions take precedence.\n{}", serde_json::to_string(&value)?),
            is_error: false, artifacts: None,
        })
}
