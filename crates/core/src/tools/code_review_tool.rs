use super::{Tool, ToolCategory, ToolDef, ToolExecutionContext, ToolResult};
use crate::{code_review, error::CoreError};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::OnceLock;
static DEF: OnceLock<ToolDef> = OnceLock::new();
const DEF_JSON: &str = include_str!("../../prompts/tools/code_review.json");
pub struct CodeReviewTool;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    action: String,
    review_id: Option<String>,
    revision: Option<String>,
    path: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
    finding: Option<code_review::FindingInput>,
}
impl Args {
    fn validate(&self) -> Result<(), CoreError> {
        let valid = match self.action.as_str() {
            "snapshot" => {
                self.review_id.is_none()
                    && self.revision.is_none()
                    && self.path.is_none()
                    && self.offset.is_none()
                    && self.limit.is_none()
                    && self.finding.is_none()
            }
            "file" => {
                self.review_id.as_ref().is_some_and(|s| !s.is_empty())
                    && self.revision.as_ref().is_some_and(|s| s.len() == 64)
                    && self.path.as_ref().is_some_and(|s| !s.is_empty())
                    && self.finding.is_none()
                    && self.limit.is_none_or(|n| (1..=400).contains(&n))
            }
            "add_finding" => {
                self.review_id.as_ref().is_some_and(|s| !s.is_empty())
                    && self
                        .finding
                        .as_ref()
                        .is_some_and(|f| code_review::validate_finding(f).is_ok())
                    && self.revision.is_none()
                    && self.path.is_none()
                    && self.offset.is_none()
                    && self.limit.is_none()
            }
            _ => false,
        };
        if !valid {
            return Err(CoreError::InvalidInput("Use snapshot with no other fields; file with review_id, revision, path and optional offset/limit; or add_finding with review_id and finding only".into()));
        }
        Ok(())
    }
}
pub(crate) fn validate_arguments(arguments: &str) -> Result<(), CoreError> {
    serde_json::from_str::<Args>(arguments)?.validate()
}
#[async_trait]
impl Tool for CodeReviewTool {
    fn name(&self) -> &str {
        "code_review"
    }
    fn description(&self) -> &str {
        &ToolDef::from_json(&DEF, DEF_JSON).description
    }
    fn parameters_schema(&self) -> Value {
        ToolDef::from_json(&DEF, DEF_JSON).parameters.clone()
    }
    fn categories(&self) -> &'static [ToolCategory] {
        &[ToolCategory::FileSystem]
    }
    fn is_read_only(&self, args: &Value) -> bool {
        args["action"] != "add_finding"
    }
    fn is_concurrency_safe(&self, _: &Value) -> bool {
        false
    }
    async fn execute(&self, context: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        let args: Args = serde_json::from_str(context.arguments)?;
        args.validate()?;
        let conversation = context
            .conversation_id
            .ok_or_else(|| CoreError::InvalidInput("Code review requires an active chat".into()))?;
        let selected = context
            .db
            .conversation_workspace(conversation)?
            .ok_or_else(|| {
                CoreError::InvalidInput("Code review requires a selected workspace".into())
            })?;
        if context.workspace.map(|workspace| &workspace.roots) != Some(&selected.roots) {
            return Err(CoreError::InvalidInput(
                "Review workspace differs from this agent's workspace; review from its owning chat"
                    .into(),
            ));
        }
        let value = if args.action == "add_finding" {
            code_review::add(
                context.db,
                conversation,
                args.review_id.as_deref().unwrap(),
                args.finding.unwrap(),
            )
            .await?
            .view(false)
        } else {
            let review = code_review::get(context.db, conversation)
                .await?
                .ok_or_else(|| {
                    CoreError::InvalidInput("Start a review from /review first".into())
                })?;
            if args.action == "snapshot" {
                review.view(false)
            } else {
                if args.review_id.as_deref() != Some(&review.id)
                    || args.revision.as_deref() != Some(&review.snapshot.revision)
                {
                    return Err(CoreError::InvalidInput(
                        "Review version changed; read snapshot again".into(),
                    ));
                }
                let file = review
                    .snapshot
                    .files
                    .iter()
                    .find(|file| Some(&file.path) == args.path.as_ref())
                    .ok_or_else(|| {
                        CoreError::InvalidInput("File is outside this review diff".into())
                    })?;
                let lines = file.patch.lines().collect::<Vec<_>>();
                let offset = args.offset.unwrap_or(0);
                let limit = args.limit.unwrap_or(200);
                json!({"reviewId":review.id,"revision":review.snapshot.revision,"path":file.path,"patch":lines.iter().skip(offset).take(limit).copied().collect::<Vec<_>>().join("\n"),"offset":offset,"totalLines":lines.len(),"nextOffset":if offset.saturating_add(limit)<lines.len(){Some(offset+limit)}else{None},"truncated":file.truncated,"binary":file.binary})
            }
        };
        Ok(ToolResult {
            call_id: context.call_id.into(),
            content: serde_json::to_string(&value)?,
            is_error: false,
            artifacts: Some(json!({"codeReview":value})),
        })
    }
}
