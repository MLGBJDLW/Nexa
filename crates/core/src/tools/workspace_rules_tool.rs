use super::{Tool, ToolCategory, ToolDef, ToolExecutionContext, ToolResult};
use crate::{error::CoreError, workspace_rules};
use async_trait::async_trait;
use serde::Deserialize;
use std::{path::PathBuf, sync::OnceLock};

static DEF: OnceLock<ToolDef> = OnceLock::new();
const DEF_JSON: &str = include_str!("../../prompts/tools/workspace_rules.json");

pub struct WorkspaceRulesTool;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    action: Action,
    path: Option<String>,
    paths: Option<Vec<String>>,
    revision: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Read,
    Acknowledge,
}

#[async_trait]
impl Tool for WorkspaceRulesTool {
    fn name(&self) -> &str {
        "workspace_rules"
    }
    fn description(&self) -> &str {
        &ToolDef::from_json(&DEF, DEF_JSON).description
    }
    fn parameters_schema(&self) -> serde_json::Value {
        ToolDef::from_json(&DEF, DEF_JSON).parameters.clone()
    }
    fn categories(&self) -> &'static [ToolCategory] {
        &[ToolCategory::FileSystem]
    }
    fn is_read_only(&self, _: &serde_json::Value) -> bool {
        true
    }
    fn is_concurrency_safe(&self, _: &serde_json::Value) -> bool {
        true
    }

    async fn execute(&self, context: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        let args: Arguments = serde_json::from_str(context.arguments)?;
        let workspace = context.workspace.ok_or_else(|| {
            CoreError::InvalidInput(
                "Choose project workspace folders before loading file rules".into(),
            )
        })?;
        if args.path.is_some() && args.paths.is_some() {
            return Err(CoreError::InvalidInput(
                "Use either path or paths, not both".into(),
            ));
        }
        let targets = args
            .path
            .into_iter()
            .chain(args.paths.unwrap_or_default())
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let rules = workspace_rules::load(workspace, &targets);
        if matches!(args.action, Action::Acknowledge) {
            let registry = context.tool_registry.ok_or_else(|| {
                CoreError::InvalidInput(
                    "Workspace rule acknowledgement needs an active tool registry".into(),
                )
            })?;
            let conversation = context.conversation_id.ok_or_else(|| {
                CoreError::InvalidInput(
                    "Workspace rule acknowledgement needs an active conversation".into(),
                )
            })?;
            let turn = context.turn_id.ok_or_else(|| {
                CoreError::InvalidInput(
                    "Workspace rule acknowledgement needs an active turn".into(),
                )
            })?;
            registry.workspace_rules.acknowledge(
                conversation,
                turn,
                &rules,
                args.revision.as_deref().ok_or_else(|| {
                    CoreError::InvalidInput(
                        "Acknowledgement requires the revision returned after reading the rules"
                            .into(),
                    )
                })?,
            )?;
        }
        Ok(ToolResult {
            call_id: context.call_id.into(),
            content: serde_json::to_string(&rules)?,
            is_error: false,
            artifacts: Some(
                serde_json::json!({"workspaceRules": rules, "acknowledged": matches!(args.action, Action::Acknowledge)}),
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::Database,
        tools::{ToolOutput, ToolRegistry},
        workspace::Workspace,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct Writer(Arc<AtomicUsize>);
    struct Reader;
    #[async_trait]
    impl Tool for Reader {
        fn name(&self) -> &str {
            "read_file"
        }
        fn description(&self) -> &str {
            "test reader"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}})
        }
        fn categories(&self) -> &'static [ToolCategory] {
            &[ToolCategory::FileSystem]
        }
        fn is_read_only(&self, _: &serde_json::Value) -> bool {
            true
        }
        async fn execute(
            &self,
            context: ToolExecutionContext<'_>,
        ) -> Result<ToolResult, CoreError> {
            Ok(ToolResult::from_output(
                context.call_id,
                false,
                ToolOutput::text("file contents"),
            ))
        }
    }
    #[async_trait]
    impl Tool for Writer {
        fn name(&self) -> &str {
            "create_file"
        }
        fn description(&self) -> &str {
            "test writer"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}})
        }
        fn categories(&self) -> &'static [ToolCategory] {
            &[ToolCategory::FileSystem]
        }
        async fn execute(
            &self,
            context: ToolExecutionContext<'_>,
        ) -> Result<ToolResult, CoreError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(ToolResult::from_output(
                context.call_id,
                false,
                ToolOutput::text("written"),
            ))
        }
    }

    #[tokio::test]
    async fn workspace_rules_guard_shared_registry_before_effect_and_refresh_after_edit() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("child")).unwrap();
        std::fs::write(root.path().join("AGENTS.md"), "Preserve user changes.").unwrap();
        std::fs::write(root.path().join("child/AGENTS.md"), "Use child tests.").unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let workspace = Workspace::validate(&[root.path().to_string_lossy().into_owned()]).unwrap();
        let mut registry = ToolRegistry::new().with_workspace(Some(workspace.clone()));
        registry.register(Box::new(Writer(count.clone())));
        registry.register(Box::new(Reader));
        registry.register(Box::new(WorkspaceRulesTool));
        let filtered = registry
            .clone()
            .with_workspace(None)
            .filtered(&["create_file".into()])
            .with_workspace(Some(workspace));
        assert!(
            filtered.contains("workspace_rules"),
            "a narrowed worker must retain the acknowledgement prerequisite"
        );
        assert!(
            !filtered.contains("read_file"),
            "the dependency must not expand general file access"
        );
        let db = Database::open_memory().unwrap();
        let context = |call, args| {
            ToolExecutionContext::new(call, args, &db, &[])
                .with_conversation_id(Some("chat"))
                .with_turn_id(Some("turn"))
        };
        let read = registry
            .execute("read_file", context("read", r#"{"path":"child/new.txt"}"#))
            .await
            .unwrap();
        assert!(!read.is_error);
        assert!(read.llm_context_content().contains("Use child tests."));
        let pending = filtered
            .execute(
                "create_file",
                context("write", r#"{"path":"child/new.txt"}"#),
            )
            .await
            .unwrap();
        assert!(pending.is_error);
        assert_eq!(count.load(Ordering::Relaxed), 0);
        assert!(pending.content.contains("Use child tests."));
        let revision = pending.artifacts.as_ref().unwrap()["workspaceRules"]["revision"]
            .as_str()
            .unwrap();
        let acknowledge =
            serde_json::json!({"action":"acknowledge","path":"child/new.txt","revision":revision})
                .to_string();
        assert!(
            !filtered
                .execute("workspace_rules", context("ack", &acknowledge))
                .await
                .unwrap()
                .is_error
        );
        let result = filtered
            .execute(
                "create_file",
                context("retry", r#"{"path":"child/new.txt"}"#),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(count.load(Ordering::Relaxed), 1);
        assert_eq!(result.llm_context_content(), "written");
        std::fs::write(root.path().join("child/AGENTS.md"), "Run updated checks.").unwrap();
        assert!(
            registry
                .execute("workspace_rules", context("stale", &acknowledge))
                .await
                .unwrap()
                .is_error
        );
        assert!(
            filtered
                .execute(
                    "create_file",
                    context("later", r#"{"path":"child/new.txt"}"#)
                )
                .await
                .unwrap()
                .is_error
        );
        assert_eq!(count.load(Ordering::Relaxed), 1);
        let other = ToolExecutionContext::new("other", r#"{"path":"new.txt"}"#, &db, &[])
            .with_conversation_id(Some("other-chat"))
            .with_turn_id(Some("turn"));
        assert!(
            filtered
                .execute("create_file", other)
                .await
                .unwrap()
                .is_error
        );
    }
}
