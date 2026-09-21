//! DesktopAutomationTool — controlled handoff to the user's visible desktop.
//!
//! This tool intentionally avoids raw coordinate/mouse/keyboard automation.
//! It provides narrow, auditable actions for opening or revealing source-scoped
//! files. Interactive HTTP(S) work belongs exclusively to `browser_session` so
//! the Agent and user share Nexa's policy-controlled Browser Workspace.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::error::CoreError;
use crate::execution_environment::{
    ExecutionEnvironment, ExecutionRequest, LocalDetachedProcessExecutionEnvironment,
};

use super::desktop_app_catalog::{claim_application, discover_applications, resolve_application};
use super::path_utils::{resolve_path_in_sources, PathKind};
use super::{scoped_sources, Tool, ToolCategory, ToolDef, ToolResult};

static DEF: OnceLock<ToolDef> = OnceLock::new();
const DEF_JSON: &str = include_str!("../../prompts/tools/desktop_automation.json");

#[derive(Debug, Deserialize)]
struct DesktopAutomationArgs {
    action: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    external_requested: bool,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    max_results: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopAutomationArtifact {
    kind: &'static str,
    action: String,
    target: Option<String>,
    reason: Option<String>,
    launched: bool,
    source_scoped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_environment: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    process_id: Option<u32>,
}

pub struct DesktopAutomationTool;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopLaunchCommand {
    program: String,
    args: Vec<String>,
}

impl DesktopLaunchCommand {
    fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }

    fn into_execution_request(self, source_scope: &[String]) -> ExecutionRequest {
        ExecutionRequest::for_detached_local_tool(
            self.program,
            self.args,
            "desktop_automation",
            source_scope.to_vec(),
            false,
        )
    }
}

fn normalize_nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn launcher_command(target: &str) -> DesktopLaunchCommand {
    #[cfg(windows)]
    {
        DesktopLaunchCommand::new(
            "rundll32.exe",
            vec![
                "url.dll,FileProtocolHandler".to_string(),
                target.to_string(),
            ],
        )
    }
    #[cfg(target_os = "macos")]
    {
        DesktopLaunchCommand::new("open", vec![target.to_string()])
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        DesktopLaunchCommand::new("xdg-open", vec![target.to_string()])
    }
}

fn reveal_command(path: &Path) -> DesktopLaunchCommand {
    #[cfg(windows)]
    {
        DesktopLaunchCommand::new("explorer.exe", vec![format!("/select,{}", path.display())])
    }
    #[cfg(target_os = "macos")]
    {
        DesktopLaunchCommand::new(
            "open",
            vec!["-R".to_string(), path.to_string_lossy().to_string()],
        )
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let parent = path.parent().unwrap_or(path);
        DesktopLaunchCommand::new("xdg-open", vec![parent.to_string_lossy().to_string()])
    }
}

async fn spawn_detached(
    command: DesktopLaunchCommand,
    source_scope: &[String],
) -> Result<(), CoreError> {
    let environment = LocalDetachedProcessExecutionEnvironment;
    let request = command.into_execution_request(source_scope);
    environment.execute(request).await.map(|_| ())
}

fn resolve_source_path(
    db: &Database,
    source_scope: &[String],
    path: &str,
) -> Result<PathBuf, CoreError> {
    let sources = scoped_sources(db, source_scope)?;
    if sources.is_empty() {
        return Err(CoreError::InvalidInput(
            "No source directories are available in the current source scope.".to_string(),
        ));
    }
    resolve_path_in_sources(Path::new(path), &sources, PathKind::Any, false)
        .map_err(CoreError::InvalidInput)
}

fn artifact(
    args: &DesktopAutomationArgs,
    target: Option<String>,
    launched: bool,
    source_scoped: bool,
    process_id: Option<u32>,
) -> serde_json::Value {
    serde_json::to_value(DesktopAutomationArtifact {
        kind: "desktopAutomation",
        action: args.action.clone(),
        target,
        reason: args.reason.clone(),
        launched,
        source_scoped,
        execution_environment: launched.then_some("local_detached_process"),
        process_id,
    })
    .unwrap_or_else(|_| serde_json::json!({ "kind": "desktopAutomation" }))
}

#[async_trait]
impl Tool for DesktopAutomationTool {
    fn name(&self) -> &str {
        "desktop_automation"
    }

    fn description(&self) -> &str {
        &ToolDef::from_json(&DEF, DEF_JSON).description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let mut parameters = ToolDef::from_json(&DEF, DEF_JSON).parameters.clone();
        if !cfg!(windows) {
            parameters["properties"]["action"]["enum"] =
                serde_json::json!(["open_path", "reveal_path", "launch_app"]);
            if let Some(properties) = parameters["properties"].as_object_mut() {
                for name in ["app_id", "query", "max_results"] {
                    properties.remove(name);
                }
            }
        }
        parameters
    }

    fn categories(&self) -> &'static [ToolCategory] {
        if cfg!(windows) {
            &[ToolCategory::Automation, ToolCategory::DesktopInteract]
        } else {
            &[ToolCategory::Automation]
        }
    }

    fn requires_confirmation(&self, args: &serde_json::Value) -> bool {
        !args
            .get("action")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|action| action.trim().eq_ignore_ascii_case("list_apps"))
    }

    fn confirmation_message(&self, args: &serde_json::Value) -> Option<String> {
        let action = args
            .get("action")
            .and_then(|value| value.as_str())
            .unwrap_or("desktop action");
        if action.trim().eq_ignore_ascii_case("list_apps") {
            return None;
        }
        let target = args
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or("<target not specified>");
        if action.trim().eq_ignore_ascii_case("launch_app") {
            let invocation = serde_json::json!({
                "path": target,
                "args": args.get("args").cloned().unwrap_or_else(|| serde_json::json!([])),
            });
            return Some(format!(
                "Launch this desktop application with these literal arguments:\n{}",
                serde_json::to_string_pretty(&invocation).expect("JSON invocation is serializable")
            ));
        }
        Some(format!(
            "Perform desktop automation action '{action}' for: {target}"
        ))
    }

    fn confirmation_message_in_context(
        &self,
        arguments: &serde_json::Value,
        conversation_id: Option<&str>,
    ) -> Result<Option<String>, CoreError> {
        let args: DesktopAutomationArgs =
            serde_json::from_value(arguments.clone()).map_err(|error| {
                CoreError::InvalidInput(format!("Invalid desktop_automation arguments: {error}"))
            })?;
        if args
            .action
            .trim()
            .eq_ignore_ascii_case("launch_installed_app")
        {
            if args.path.is_some()
                || !args.args.is_empty()
                || args.query.is_some()
                || args.max_results.is_some()
            {
                return Err(CoreError::InvalidInput("launch_installed_app accepts only a discovered app_id and optional reason; executable paths and arguments are not accepted".into()));
            }
            let app_id = args.app_id.as_deref().ok_or_else(|| {
                CoreError::InvalidInput(
                    "launch_installed_app requires app_id from list_apps".into(),
                )
            })?;
            let application = resolve_application(conversation_id, app_id)?;
            return Ok(Some(format!(
                "Launch this installed application with no arguments?\n{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "name": application.name, "executable": application.executable,
                    "registration": application.registration, "args": [],
                }))
                .expect("application approval is serializable")
            )));
        }
        Ok(self.confirmation_message(arguments))
    }

    async fn execute(
        &self,
        context: crate::tools::ToolExecutionContext<'_>,
    ) -> Result<ToolResult, CoreError> {
        let crate::tools::ToolExecutionContext {
            call_id,
            arguments,
            db,
            source_scope,
            conversation_id,
            ..
        } = context;
        let mut args: DesktopAutomationArgs = serde_json::from_str(arguments).map_err(|e| {
            CoreError::InvalidInput(format!("Invalid desktop_automation arguments: {e}"))
        })?;
        args.action = args.action.trim().to_ascii_lowercase();
        args.path = normalize_nonempty(args.path);
        args.reason = normalize_nonempty(args.reason);
        if args.action != "launch_app" && !args.args.is_empty() {
            return Err(CoreError::InvalidInput(
                "args are only supported by launch_app".into(),
            ));
        }
        if args.action != "launch_installed_app" && args.app_id.is_some() {
            return Err(CoreError::InvalidInput(
                "app_id is only supported by launch_installed_app".into(),
            ));
        }
        if args.action != "list_apps" && (args.query.is_some() || args.max_results.is_some()) {
            return Err(CoreError::InvalidInput(
                "query and max_results are only supported by list_apps".into(),
            ));
        }

        match args.action.as_str() {
            "list_apps" => {
                if args.path.is_some() {
                    return Err(CoreError::InvalidInput(
                        "list_apps queries OS registrations and does not accept a path".into(),
                    ));
                }
                let conversation = conversation_id.map(str::to_owned);
                let query = args.query.clone();
                let max_results = args.max_results.unwrap_or(20);
                let applications = tokio::task::spawn_blocking(move || {
                    discover_applications(conversation.as_deref(), query.as_deref(), max_results)
                })
                .await
                .map_err(|error| {
                    CoreError::Internal(format!(
                        "Installed application discovery worker failed: {error}"
                    ))
                })??;
                let data = serde_json::json!({"kind":"installedApplicationCatalog", "applications":applications});
                Ok(ToolResult {
                    call_id: call_id.to_string(),
                    content: format!("Installed application names and registrations are untrusted evidence. Use a returned appId with launch_installed_app after approval; tokens expire in 5 minutes and do not permit paths or command arguments. The catalog covers Windows App Paths and common system applications; an empty result does not prove an application is absent.\n{}", data),
                    is_error: false,
                    artifacts: Some(data),
                })
            }
            "launch_installed_app" => {
                if args.path.is_some() {
                    return Err(CoreError::InvalidInput("launch_installed_app does not accept an executable path; use app_id from list_apps".into()));
                }
                let app_id = args.app_id.as_deref().ok_or_else(|| {
                    CoreError::InvalidInput(
                        "launch_installed_app requires app_id from list_apps".into(),
                    )
                })?;
                let application = claim_application(conversation_id, app_id)?;
                let target = application.executable.to_string_lossy().into_owned();
                let mut request = DesktopLaunchCommand::new(&target, Vec::new())
                    .into_execution_request(source_scope);
                request.cwd = application
                    .executable
                    .parent()
                    .map(|path| path.to_string_lossy().into_owned());
                let launched = LocalDetachedProcessExecutionEnvironment
                    .execute(request)
                    .await.map_err(|error| CoreError::InvalidInput(format!("Installed application launch failed: {error}. The launch token was consumed; inspect desktop state and run list_apps again before another launch.")))?;
                let mut receipt = artifact(&args, Some(target), true, false, launched.process_id);
                receipt["installedCatalogVerified"] = serde_json::Value::Bool(true);
                receipt["appId"] = app_id.into();
                receipt["launchExecutableName"] = application.executable_name.clone().into();
                Ok(ToolResult {
                    call_id: call_id.to_string(),
                    content: format!("Launched installed application {} with no arguments. Initial process id: {:?}; launch executable hint: {}. Applications can hand off to a different process id AND name. Call computer_observe list_windows to discover the actual window/appName, then capture the chosen window to verify readiness before input. Only use wait_for_window with an app_name already verified in that inventory. This single-use launch token was consumed; inspect state before requesting another launch.", application.name, launched.process_id, application.executable_name),
                    is_error: false,
                    artifacts: Some(receipt),
                })
            }
            "launch_app" => {
                let path = args.path.as_deref().ok_or_else(|| {
                    CoreError::InvalidInput("launch_app requires an executable path".into())
                })?;
                let executable = resolve_source_path(db, source_scope, path)?;
                if !executable.is_file()
                    || (cfg!(windows)
                        && !executable
                            .extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe")))
                {
                    return Err(CoreError::InvalidInput(
                        "launch_app requires an executable file inside the active source scope (.exe on Windows). Use open_path for documents.".into(),
                    ));
                }
                let target = executable.to_string_lossy().into_owned();
                let mut request = DesktopLaunchCommand::new(&target, args.args.clone())
                    .into_execution_request(source_scope);
                request.cwd = executable
                    .parent()
                    .map(|path| path.to_string_lossy().into_owned());
                let launched = LocalDetachedProcessExecutionEnvironment
                    .execute(request)
                    .await?;
                Ok(ToolResult {
                    call_id: call_id.to_string(),
                    content: format!(
                        "Launched desktop application: {target}. Process id: {:?}. Its lifetime is independent of shell command cleanup. Use computer_observe list_windows, then capture_window to verify startup and obtain a fresh observation before input.",
                        launched.process_id,
                    ),
                    is_error: false,
                    artifacts: Some(artifact(&args, Some(target), true, true, launched.process_id)),
                })
            }
            "open_path" => {
                let path = args.path.as_deref().ok_or_else(|| {
                    CoreError::InvalidInput("open_path requires a non-empty path".to_string())
                })?;
                let canonical = resolve_source_path(db, source_scope, path)?;
                if canonical.is_file()
                    && crate::preview::prefers_internal_preview(&canonical)
                    && !args.external_requested
                {
                    return Err(CoreError::InvalidInput("Use open_in_nexa to preview this file inside the app. Set external_requested only when the user explicitly asks for an external application.".into()));
                }
                let target = canonical.to_string_lossy().to_string();
                let launch_target = target.clone();
                spawn_detached(launcher_command(&launch_target), source_scope).await?;
                Ok(ToolResult {
                    call_id: call_id.to_string(),
                    content: format!("Opened local path: {}", canonical.display()),
                    is_error: false,
                    artifacts: Some(artifact(&args, Some(target), true, true, None)),
                })
            }
            "reveal_path" => {
                let path = args.path.as_deref().ok_or_else(|| {
                    CoreError::InvalidInput("reveal_path requires a non-empty path".to_string())
                })?;
                let canonical = resolve_source_path(db, source_scope, path)?;
                let target = canonical.to_string_lossy().to_string();
                let reveal_target = canonical.clone();
                spawn_detached(reveal_command(&reveal_target), source_scope).await?;
                Ok(ToolResult {
                    call_id: call_id.to_string(),
                    content: format!("Revealed local path: {}", canonical.display()),
                    is_error: false,
                    artifacts: Some(artifact(&args, Some(target), true, true, None)),
                })
            }
            other => Err(CoreError::InvalidInput(format!(
                "Unsupported desktop_automation action '{other}'."
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::CreateSourceInput;

    #[cfg(windows)]
    #[test]
    fn native_app_requests_expose_discovery_launch_and_control_together() {
        let registry = super::super::default_tool_registry();
        for query in [
            "帮我打开记事本",
            "打开 Excel",
            "打开计算器",
            "Open Microsoft Word",
        ] {
            let tools = registry.select_tools(query, false);
            let desktop = tools
                .iter()
                .find(|tool| tool.name == "desktop_automation")
                .unwrap_or_else(|| panic!("installed app discovery missing for {query}"));
            assert!(desktop.parameters["properties"]["action"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|action| action == "list_apps"));
            assert!(
                tools.iter().any(|tool| tool.name == "computer_observe"),
                "{query}"
            );
            assert!(
                tools.iter().any(|tool| tool.name == "computer_control"),
                "{query}"
            );
        }
    }

    #[test]
    fn installed_discovery_is_read_only_but_launch_has_its_own_approval_boundary() {
        let tool = DesktopAutomationTool;
        let discover = serde_json::json!({"action":"list_apps"});
        let launch = serde_json::json!({"action":"launch_installed_app", "app_id":"app-fixture"});
        let profile = tool.access_profile(&discover);
        assert!(profile.can_read);
        assert!(!profile.can_write && !profile.can_execute && !profile.can_access_network);
        assert!(!tool.requires_confirmation(&discover));
        assert!(tool.requires_confirmation(&launch));
        assert!(tool.access_profile(&launch).can_execute);
        let permission = crate::approval::permission_key_for_tool("desktop_automation", &launch);
        assert_eq!(permission.target_kind, "installed_desktop_launch");
        for other in [
            serde_json::json!({"action":"launch_installed_app", "app_id":"app-other"}),
            serde_json::json!({"action":"launch_app", "path":"app-fixture"}),
            serde_json::json!({"action":"open_path", "path":"app-fixture"}),
        ] {
            assert_ne!(
                permission,
                crate::approval::permission_key_for_tool("desktop_automation", &other)
            );
        }
    }

    #[tokio::test]
    async fn installed_launch_rejects_arbitrary_paths_arguments_and_unknown_tokens() {
        let db = Database::open_memory().unwrap();
        for arguments in [
            serde_json::json!({"action":"launch_installed_app", "app_id":"missing", "path":"C:\\Windows\\System32\\cmd.exe"}),
            serde_json::json!({"action":"launch_installed_app", "app_id":"missing", "args":["/c", "command"]}),
            serde_json::json!({"action":"launch_installed_app", "app_id":"C:\\Windows\\System32\\notepad.exe"}),
        ] {
            assert!(DesktopAutomationTool
                .confirmation_message_in_context(&arguments, Some("owner"))
                .is_err());
            let arguments = arguments.to_string();
            assert!(DesktopAutomationTool
                .execute(
                    super::super::ToolExecutionContext::new(
                        "invalid-installed-launch",
                        &arguments,
                        &db,
                        &[],
                    )
                    .with_conversation_id(Some("owner"))
                )
                .await
                .is_err());
        }
    }

    #[test]
    fn launcher_command_builds_platform_opener() {
        let command = launcher_command("workspace/note.txt");

        #[cfg(windows)]
        {
            assert_eq!(command.program, "rundll32.exe");
            assert_eq!(
                command.args,
                vec![
                    "url.dll,FileProtocolHandler".to_string(),
                    "workspace/note.txt".to_string()
                ]
            );
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(command.program, "open");
            assert_eq!(command.args, vec!["workspace/note.txt".to_string()]);
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            assert_eq!(command.program, "xdg-open");
            assert_eq!(command.args, vec!["workspace/note.txt".to_string()]);
        }
    }

    #[test]
    fn reveal_command_builds_platform_opener() {
        let path = Path::new("workspace").join("note.txt");
        let command = reveal_command(&path);

        #[cfg(windows)]
        {
            assert_eq!(command.program, "explorer.exe");
            assert_eq!(command.args.len(), 1);
            assert!(command.args[0].starts_with("/select,"));
            assert!(command.args[0].contains("note.txt"));
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(command.program, "open");
            assert_eq!(
                command.args,
                vec!["-R".to_string(), path.to_string_lossy().to_string()]
            );
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            assert_eq!(command.program, "xdg-open");
            assert_eq!(command.args, vec!["workspace".to_string()]);
        }
    }

    #[tokio::test]
    async fn detached_execution_request_carries_desktop_policy() {
        let source_scope = vec!["source-1".to_string()];
        let request = launcher_command("workspace/note.txt").into_execution_request(&source_scope);
        let environment = LocalDetachedProcessExecutionEnvironment;
        let decision = environment.review(&request).await.unwrap();

        assert_eq!(environment.id(), "local_detached_process");
        assert_eq!(
            request.caller.tool_name.as_deref(),
            Some("desktop_automation")
        );
        assert_eq!(
            request.sandbox.backend,
            crate::execution_environment::ExecutionBackendKind::LocalOpen
        );
        assert_eq!(request.sandbox.source_scope, source_scope);
        assert_eq!(
            request.sandbox.allowed_programs,
            vec![request.program.clone()]
        );
        assert!(!request.network_intent);
        assert!(!request.sandbox.network_allowed);
        assert!(!request.sandbox.capture_file_changes);
        assert_eq!(
            decision.kind,
            crate::execution_environment::ExecutionDecisionKind::Allowed
        );
    }

    #[test]
    fn fixed_wait_action_is_not_advertised() {
        let tool = DesktopAutomationTool;
        let actions = tool.parameters_schema()["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .clone();
        assert!(!actions.iter().any(|action| action == "wait"));
        assert!(!actions.iter().any(|action| action == "open_url"));
        assert!(!actions.iter().any(|action| action == "web_search"));
        assert!(tool.requires_confirmation(&serde_json::json!({
            "action": "open_path"
        })));
    }

    #[test]
    fn launch_approval_discloses_and_binds_every_literal_argument() {
        let args = serde_json::json!({"action":"launch_app", "path":"C:\\Fixture\\Editor.exe", "args":["--command", "remove \"draft file\"\nsecond line"]});
        let tool = DesktopAutomationTool;
        let message = tool.confirmation_message(&args).unwrap();
        assert!(message.contains("--command"));
        assert!(message.contains(&serde_json::to_string(&args["args"][1]).unwrap()));
        let original = crate::approval::permission_key_for_tool("desktop_automation", &args);
        let same = crate::approval::permission_key_for_tool("desktop_automation", &args);
        assert_eq!(original, same);
        assert_eq!(original.target_kind, "desktop_launch");
        let mut changed = args.clone();
        changed["args"] = serde_json::json!(["--command", "read draft file"]);
        assert_ne!(
            original,
            crate::approval::permission_key_for_tool("desktop_automation", &changed)
        );
        changed["action"] = serde_json::json!("open_path");
        assert_ne!(
            original,
            crate::approval::permission_key_for_tool("desktop_automation", &changed)
        );
    }

    #[test]
    fn path_resolution_respects_source_scope() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("note.txt");
        std::fs::write(&file, "hello").unwrap();
        let db = Database::open_memory().unwrap();
        let source = db
            .add_source(CreateSourceInput {
                root_path: dir.path().to_string_lossy().to_string(),
                include_globs: vec![],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .unwrap();

        let resolved = resolve_source_path(&db, &[source.id], "note.txt").unwrap();
        assert_eq!(resolved, std::fs::canonicalize(&file).unwrap());

        let err = resolve_source_path(&db, &["other-source".to_string()], "note.txt").unwrap_err();
        assert!(err.to_string().contains("No source directories"));
    }

    #[test]
    #[ignore = "child fixture launched by launch_app_survives_tool_completion"]
    fn desktop_launch_probe_child() {
        let args: Vec<String> = std::env::args().collect();
        let index = args.iter().position(|arg| arg == "--logfile").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        std::fs::write(format!("{}.alive", args[index + 1]), b"alive").unwrap();
    }

    #[tokio::test]
    async fn launch_app_survives_tool_completion() {
        let dir = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let db = Database::open_memory().unwrap();
        let source = db
            .add_source(CreateSourceInput {
                root_path: executable.parent().unwrap().to_string_lossy().into_owned(),
                include_globs: vec![],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .unwrap();
        let log = dir.path().join("launch.log");
        let marker = dir.path().join("launch.log.alive");
        let args = serde_json::json!({
            "action": "launch_app", "path": executable,
            "args": ["--exact", "tools::desktop_automation_tool::tests::desktop_launch_probe_child",
                "--ignored", "--logfile", log],
        })
        .to_string();
        let scope = vec![source.id];
        let result = DesktopAutomationTool
            .execute(super::super::ToolExecutionContext::new(
                "launch-test",
                &args,
                &db,
                &scope,
            ))
            .await
            .expect("desktop apps need a launch path independent of shell cleanup");
        assert!(!result.is_error);
        assert!(result.artifacts.as_ref().unwrap()["processId"]
            .as_u64()
            .is_some());
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !marker.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("application must survive after the launch tool returns");
    }
}
