use super::{
    file_access_policy, path_utils::resolve_existing_file_for_file_access, Tool, ToolCategory,
    ToolDef, ToolExecutionContext, ToolResult,
};
use crate::error::CoreError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

static DEF: OnceLock<ToolDef> = OnceLock::new();
const DEF_JSON: &str = include_str!("../../prompts/tools/open_in_nexa.json");

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewOpenRequest {
    pub path: String,
    pub line: Option<usize>,
    pub conversation_id: Option<String>,
    pub call_id: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewOpenReceipt {
    pub path: String,
    pub kind: String,
    pub display_mode: String,
    pub warning: Option<String>,
}
#[async_trait]
pub trait NexaPreviewHost: Send + Sync {
    async fn open(&self, request: PreviewOpenRequest) -> Result<PreviewOpenReceipt, String>;
}
#[derive(Default)]
pub struct OpenInNexaTool {
    host: Option<Arc<dyn NexaPreviewHost>>,
}
impl OpenInNexaTool {
    pub fn new(host: Arc<dyn NexaPreviewHost>) -> Self {
        Self { host: Some(host) }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    path: String,
    line: Option<usize>,
}

#[async_trait]
impl Tool for OpenInNexaTool {
    fn name(&self) -> &str {
        "open_in_nexa"
    }
    fn description(&self) -> &str {
        &ToolDef::from_json(&DEF, DEF_JSON).description
    }
    fn parameters_schema(&self) -> serde_json::Value {
        ToolDef::from_json(&DEF, DEF_JSON).parameters.clone()
    }
    fn categories(&self) -> &'static [ToolCategory] {
        &[ToolCategory::Core, ToolCategory::FileSystem]
    }
    fn is_concurrency_safe(&self, _: &serde_json::Value) -> bool {
        false
    }
    fn resource_keys(&self, _: &serde_json::Value) -> Vec<String> {
        vec!["nexa:preview".into()]
    }
    async fn execute(&self, context: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        let args: Arguments = serde_json::from_str(context.arguments).map_err(|error| {
            CoreError::InvalidInput(format!("Invalid open_in_nexa arguments: {error}"))
        })?;
        if args.path.trim().is_empty() || args.line == Some(0) {
            return Err(CoreError::InvalidInput(
                "Provide a file path and an optional positive line number".into(),
            ));
        }
        let host=self.host.as_ref().ok_or_else(||CoreError::InvalidInput("This runtime has no Nexa preview surface. Do not silently fall back to an external application.".into()))?;
        let db = context.db.clone();
        let scope = context.source_scope.to_vec();
        let canonical = tokio::task::spawn_blocking(move || {
            let policy = file_access_policy(&db, &scope)?;
            resolve_existing_file_for_file_access(
                &PathBuf::from(&args.path),
                &policy.sources,
                policy.allow_unregistered_absolute_paths,
            )
            .map_err(CoreError::InvalidInput)
        })
        .await
        .map_err(|error| CoreError::Internal(error.to_string()))??;
        let request = PreviewOpenRequest {
            path: canonical.to_string_lossy().into_owned(),
            line: args.line,
            conversation_id: context.conversation_id.map(str::to_string),
            call_id: context.call_id.into(),
        };
        let operation = host.open(request);
        let receipt=if let Some(cancel)=context.cancel_token {tokio::select!{biased;_=cancel.cancelled()=>Err("Nexa preview request was cancelled".into()),result=operation=>result}}else{operation.await}.map_err(CoreError::InvalidInput)?;
        Ok(ToolResult {
            call_id: context.call_id.into(),
            content: format!(
                "Opened in Nexa: {}\nPreview: {} ({}){}",
                receipt.path,
                receipt.kind,
                receipt.display_mode,
                receipt
                    .warning
                    .as_ref()
                    .map(|warning| format!("\nNote: {warning}"))
                    .unwrap_or_default()
            ),
            is_error: false,
            artifacts: Some(serde_json::json!({"kind":"nexaPreview","receipt":receipt})),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app_settings::ShellAccessMode, db::Database, sources::CreateSourceInput};
    #[derive(Default)]
    struct Host {
        requests: std::sync::Mutex<Vec<PreviewOpenRequest>>,
    }
    #[async_trait]
    impl NexaPreviewHost for Host {
        async fn open(&self, request: PreviewOpenRequest) -> Result<PreviewOpenReceipt, String> {
            let receipt = PreviewOpenReceipt {
                path: request.path.clone(),
                kind: "text".into(),
                display_mode: "preview".into(),
                warning: None,
            };
            self.requests.lock().unwrap().push(request);
            Ok(receipt)
        }
    }
    #[tokio::test]
    async fn open_in_nexa_resolves_authorized_paths_and_waits_for_the_host_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let allowed = dir.path().join("allowed");
        std::fs::create_dir(&allowed).unwrap();
        std::fs::write(allowed.join("note.md"), "hello").unwrap();
        std::fs::write(dir.path().join("outside.md"), "private").unwrap();
        let db = Database::open_memory().unwrap();
        let mut cfg = db.load_app_config().unwrap();
        cfg.shell_access_mode = ShellAccessMode::Restricted;
        db.save_app_config(&cfg).unwrap();
        let source = db
            .add_source(CreateSourceInput {
                root_path: allowed.to_string_lossy().into(),
                include_globs: vec![],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .unwrap();
        let host = Arc::new(Host::default());
        let tool = OpenInNexaTool::new(host.clone());
        let scope = vec![source.id];
        let result = tool
            .execute(ToolExecutionContext::new(
                "call",
                r#"{"path":"note.md","line":1}"#,
                &db,
                &scope,
            ))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Opened in Nexa"));
        assert_eq!(
            host.requests.lock().unwrap()[0].path,
            std::fs::canonicalize(allowed.join("note.md"))
                .unwrap()
                .to_string_lossy()
        );
        let outside = serde_json::json!({"path":dir.path().join("outside.md")}).to_string();
        assert!(tool
            .execute(ToolExecutionContext::new("denied", &outside, &db, &scope))
            .await
            .is_err());
        assert_eq!(host.requests.lock().unwrap().len(), 1);
        let cancellation = tokio_util::sync::CancellationToken::new();
        cancellation.cancel();
        let mut context =
            ToolExecutionContext::new("cancelled", r#"{"path":"note.md"}"#, &db, &scope);
        context.cancel_token = Some(&cancellation);
        assert!(tool.execute(context).await.is_err());
        assert_eq!(host.requests.lock().unwrap().len(), 1);
        let registry = crate::tools::default_tool_registry();
        assert!(registry.contains("open_in_nexa"));
        assert!(
            crate::package_host::PackageRuntimeAssembler::database_builtin(&db)
                .unwrap()
                .assemble_tool_registry(registry)
                .unwrap()
                .tools
                .contains("open_in_nexa")
        );
    }
}
