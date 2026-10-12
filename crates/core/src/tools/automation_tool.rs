//! Chat adapter over the existing durable workflow scheduler. No timer lives here.

use std::sync::OnceLock;

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::CoreError;
use crate::workflow_automation::{
    SaveWorkflowAutomationInput, WorkflowAutomation, WorkflowAutomationApprovalPolicy,
    WorkflowAutomationTrigger,
};
use crate::workflow_execution::prepare_workflow_automation_save;
use crate::workflow_scheduler::{preview_workflow_cron_schedule, WorkflowScheduleWorkspacePolicy};

use super::{scoped_sources, Tool, ToolCategory, ToolDef, ToolExecutionContext, ToolResult};

static DEF: OnceLock<ToolDef> = OnceLock::new();
pub struct AutomationTool;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Args {
    action: String,
    id: Option<String>,
    request_id: Option<String>,
    expected_revision: Option<i64>,
    name: Option<String>,
    prompt: Option<String>,
    cron: Option<String>,
    timezone: Option<String>,
    workflow_template_id: Option<String>,
    source_ids: Option<Vec<String>>,
    allowed_tools: Option<Vec<String>>,
    require_approval: Option<bool>,
    enabled: Option<bool>,
    workspace_policy: Option<WorkflowScheduleWorkspacePolicy>,
    before: Option<String>,
    limit: Option<usize>,
}

fn required<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, CoreError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CoreError::InvalidInput(format!("automation requires {field}")))
}

fn visible(task: &WorkflowAutomation, project: Option<&str>, sources: &[String]) -> bool {
    matches!(task.trigger, WorkflowAutomationTrigger::Schedule { .. })
        && task.schedule_config.execution_policy.project_id.as_deref() == project
        && task.source_scope.iter().all(|id| sources.contains(id))
        // Legacy unscoped schedules may read all sources, so a narrowed chat
        // cannot acquire them just because the saved list happens to be empty.
        && (!task.source_scope.is_empty() || task.approval_policy.allowed_tools.iter().all(|tool| source_independent(tool)))
}

fn source_independent(tool: &str) -> bool {
    matches!(tool, "web_search" | "fetch_url")
}

fn as_input(task: &WorkflowAutomation) -> SaveWorkflowAutomationInput {
    SaveWorkflowAutomationInput {
        id: Some(task.id.clone()),
        name: task.name.clone(),
        description: task.description.clone(),
        workflow_template_id: task.workflow_template_id.clone(),
        prompt: task.prompt.clone(),
        trigger: task.trigger.clone(),
        source_scope: task.source_scope.clone(),
        approval_policy: task.approval_policy.clone(),
        enabled: task.enabled,
    }
}

#[async_trait]
impl Tool for AutomationTool {
    fn name(&self) -> &str {
        "automation"
    }
    fn description(&self) -> &str {
        &ToolDef::from_json(&DEF, include_str!("../../prompts/tools/automation.json")).description
    }
    fn parameters_schema(&self) -> Value {
        ToolDef::from_json(&DEF, include_str!("../../prompts/tools/automation.json"))
            .parameters
            .clone()
    }
    fn categories(&self) -> &'static [ToolCategory] {
        &[ToolCategory::Core]
    }
    fn is_read_only(&self, args: &Value) -> bool {
        matches!(
            args["action"].as_str(),
            Some("list" | "get" | "preview" | "history")
        )
    }
    fn requires_confirmation(&self, args: &Value) -> bool {
        !self.is_read_only(args)
    }
    fn is_concurrency_safe(&self, args: &Value) -> bool {
        self.is_read_only(args)
    }
    fn resource_keys(&self, _: &Value) -> Vec<String> {
        vec!["workflow:automations".into()]
    }
    fn confirmation_message(&self, args: &Value) -> Option<String> {
        Some(format!(
            "{} scheduled task '{}'. Saved tool grants apply to future runs while Nexa is running.",
            args["action"].as_str().unwrap_or("Change"),
            args["name"]
                .as_str()
                .or(args["id"].as_str())
                .unwrap_or("new task")
        ))
    }
    async fn execute(&self, ctx: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        validate_action_fields(ctx.arguments)?;
        let args: Args = serde_json::from_str(ctx.arguments)?;
        let result = execute(&ctx, args)?;
        Ok(ToolResult {
            call_id: ctx.call_id.into(),
            content: serde_json::to_string(&result)?,
            is_error: false,
            artifacts: Some(json!({"kind":"automationResult", "result":result})),
        })
    }
}

fn validate_action_fields(arguments: &str) -> Result<(), CoreError> {
    let args: Value = serde_json::from_str(arguments)?;
    let action = args["action"].as_str().unwrap_or("");
    let allowed: &[&str] = match action {
        "list" => &[],
        "get" => &["id"],
        "preview" => &["cron", "timezone", "limit"],
        "history" => &["id", "before", "limit"],
        "pause" | "resume" | "delete" => &["id", "expectedRevision"],
        "create" | "update" => &[
            "id",
            "requestId",
            "expectedRevision",
            "name",
            "prompt",
            "cron",
            "timezone",
            "workflowTemplateId",
            "sourceIds",
            "allowedTools",
            "requireApproval",
            "enabled",
            "workspacePolicy",
        ],
        _ => return Err(CoreError::InvalidInput("Unknown automation action".into())),
    };
    for key in args.as_object().into_iter().flat_map(|value| value.keys()) {
        if !matches!(
            key.as_str(),
            "action" | "wait_for_previous" | "waitForPrevious"
        ) && !allowed.contains(&key.as_str())
        {
            return Err(CoreError::InvalidInput(format!(
                "{key} is not accepted for automation {action}"
            )));
        }
    }
    if (action == "create" && (args.get("id").is_some() || args.get("expectedRevision").is_some()))
        || (action == "update" && args.get("requestId").is_some())
    {
        return Err(CoreError::InvalidInput(
            "Use requestId only for create; id and expectedRevision only for existing tasks".into(),
        ));
    }
    Ok(())
}

fn execute(ctx: &ToolExecutionContext<'_>, args: Args) -> Result<Value, CoreError> {
    if args.action == "preview" {
        return Ok(serde_json::to_value(preview_workflow_cron_schedule(
            required(args.cron.as_deref(), "cron")?,
            required(args.timezone.as_deref(), "timezone")?,
            Utc::now(),
            args.limit.unwrap_or(5).clamp(1, 20),
        )?)?);
    }
    let conversation_id = required(ctx.conversation_id, "an active conversation")?;
    let project = ctx.db.get_conversation(conversation_id)?.project_id;
    let mut sources: Vec<_> = scoped_sources(ctx.db, ctx.source_scope)?
        .into_iter()
        .map(|source| source.id)
        .collect();
    if let Some(project_id) = project.as_deref() {
        let project_sources = ctx
            .db
            .get_project(project_id)?
            .source_scope
            .unwrap_or_default();
        sources.retain(|id| project_sources.contains(id));
    }
    if args.action == "list" {
        return Ok(
            json!({"tasks":ctx.db.list_workflow_automations()?.into_iter()
            .filter(|task| visible(task, project.as_deref(), &sources)).collect::<Vec<_>>() }),
        );
    }
    let existing = if args.action == "create" {
        None
    } else {
        let task = ctx
            .db
            .get_workflow_automation(required(args.id.as_deref(), "id")?)?;
        if !visible(&task, project.as_deref(), &sources) {
            return Err(CoreError::InvalidInput(
                "Scheduled task is outside this conversation's project/source scope".into(),
            ));
        }
        Some(task)
    };
    if args.action == "get" {
        return Ok(serde_json::to_value(existing)?);
    }
    if args.action == "history" {
        return Ok(
            json!({"runs":ctx.db.list_workflow_run_history(&existing.as_ref().unwrap().id,
            args.before.as_deref(), args.limit.unwrap_or(25))?}),
        );
    }
    if let Some(task) = &existing {
        if args.expected_revision != Some(task.definition_revision) {
            return Err(CoreError::Conflict(
                "Get the task and pass its current definitionRevision as expectedRevision".into(),
            ));
        }
        if args.action == "delete" {
            ctx.db
                .delete_workflow_automation_at_revision(&task.id, task.definition_revision)?;
            return Ok(json!({"id":task.id,"deleted":true}));
        }
        if args.action == "pause" {
            // Stopping a schedule must still work after a tool/provider/package
            // becomes unavailable. No new grant is needed to disable it.
            let mut input = as_input(task);
            input.enabled = false;
            let paused = ctx.db.save_authored_workflow_automation(
                &input,
                &task.schedule_config,
                None,
                args.expected_revision,
            )?;
            return Ok(json!({"task":paused,"created":false}));
        }
        if !matches!(&task.trigger, WorkflowAutomationTrigger::Schedule { .. }) {
            return Err(CoreError::InvalidInput(
                "automation edits cron tasks; use Workflows to edit manual/folder definitions"
                    .into(),
            ));
        }
        if task.recipe.is_some() && args.prompt.is_some() {
            return Err(CoreError::InvalidInput("This task uses a structured recipe. Edit its prompt in Workflows to preserve typed inputs".into()));
        }
    }
    if !matches!(
        args.action.as_str(),
        "create" | "update" | "pause" | "resume"
    ) {
        return Err(CoreError::InvalidInput("Unknown automation action".into()));
    }
    let creating = args.action == "create";
    let replace_workspace = args.source_ids.is_some() || args.workspace_policy.is_some();
    let mut input = if let Some(task) = &existing {
        as_input(task)
    } else {
        let request = required(args.request_id.as_deref(), "requestId")?;
        if request.len() > 120 {
            return Err(CoreError::InvalidInput(
                "requestId exceeds 120 bytes".into(),
            ));
        }
        let key = format!(
            "{}\0{request}",
            project.as_deref().unwrap_or(conversation_id)
        );
        SaveWorkflowAutomationInput {
            id: Some(format!("chat-{}", blake3::hash(key.as_bytes()).to_hex())),
            name: required(args.name.as_deref(), "name")?.into(),
            description: String::new(),
            workflow_template_id: "report_brief".into(),
            prompt: required(args.prompt.as_deref(), "prompt")?.into(),
            trigger: WorkflowAutomationTrigger::Schedule {
                cron: required(args.cron.as_deref(), "cron")?.into(),
            },
            source_scope: sources.clone(),
            approval_policy: WorkflowAutomationApprovalPolicy {
                require_before_run: false,
                ..Default::default()
            },
            enabled: true,
        }
    };
    let mut config = existing
        .as_ref()
        .map(|task| task.schedule_config.clone())
        .unwrap_or_default();
    if creating {
        config.timezone = required(args.timezone.as_deref(), "timezone")?.into();
    }
    if let Some(value) = args.name {
        input.name = value;
    }
    if let Some(value) = args.prompt {
        input.prompt = value;
    }
    if let Some(value) = args.workflow_template_id {
        input.workflow_template_id = value;
    }
    if let Some(value) = args.cron {
        input.trigger = WorkflowAutomationTrigger::Schedule { cron: value };
    }
    if let Some(value) = args.timezone {
        config.timezone = value;
    }
    if let Some(value) = args.source_ids {
        input.source_scope = value;
    }
    if input.source_scope.iter().any(|id| !sources.contains(id)) {
        return Err(CoreError::InvalidInput(
            "sourceIds must be within the current conversation sources".into(),
        ));
    }
    if let Some(value) = args.allowed_tools {
        input.approval_policy.allowed_tools = value;
    }
    if let Some(value) = args.require_approval {
        input.approval_policy.require_before_run = value;
    }
    if let Some(value) = args.enabled {
        input.enabled = value;
    }
    if args.action == "pause" {
        input.enabled = false;
    }
    if args.action == "resume" {
        input.enabled = true;
    }
    config.execution_policy.project_id = project;
    if let Some(value) = args.workspace_policy {
        config.execution_policy.workspace_policy = value;
        if value == WorkflowScheduleWorkspacePolicy::IsolatedPatch {
            config.execution_policy.orchestration_profile = "codeUltra".into();
        }
    }
    let registry = ctx.tool_registry.ok_or_else(|| {
        CoreError::InvalidInput("automation requires the active tool registry".into())
    })?;
    for tool in &input.approval_policy.allowed_tools {
        if registry.get(tool).is_none() {
            return Err(CoreError::InvalidInput(format!(
                "Scheduled tool '{tool}' is not available in this conversation"
            )));
        }
    }
    if input.source_scope.is_empty()
        && input
            .approval_policy
            .allowed_tools
            .iter()
            .any(|tool| !source_independent(tool))
    {
        return Err(CoreError::InvalidInput("Schedules using local tools require current sources to freeze their future access scope; web_search/fetch_url can run without sources".into()));
    }
    let snapshot = crate::package_host::database_backed_builtin_package_host_snapshot(ctx.db)
        .map_err(|error| CoreError::InvalidInput(error.to_string()))?;
    if !snapshot.runtime_components().iter().any(|component| {
        component.kind == crate::package_host::PackageSurfaceKind::Workflow
            && component.id == input.workflow_template_id
    }) {
        return Err(CoreError::InvalidInput(
            "Workflow template is disabled or unavailable".into(),
        ));
    }
    input.name = input.name.trim().to_string();
    input.prompt = input.prompt.trim().to_string();
    input.workflow_template_id = input.workflow_template_id.trim().to_string();
    input.source_scope.sort();
    input.source_scope.dedup();
    let mut prepared = prepare_workflow_automation_save(ctx.db, input, Some(config))?;
    // Core preparation may expand an empty project scope. Its final snapshot,
    // not only the model's input, must remain within the caller's authority.
    if prepared
        .input
        .source_scope
        .iter()
        .any(|id| !sources.contains(id))
    {
        return Err(CoreError::InvalidInput("Prepared schedule would exceed this conversation's sources; provide explicit sourceIds from the current scope".into()));
    }
    prepared.input.source_scope.sort();
    prepared.input.source_scope.dedup();
    if let Some(previous) = &existing {
        // Name/cron edits and resume cannot silently approve an Agent route or
        // Source-root change that happened after this task was created.
        let saved = &previous.schedule_config.execution_policy;
        let execution = &mut prepared.schedule_config.execution_policy;
        execution.agent_config_id = saved.agent_config_id.clone();
        execution.provider = saved.provider.clone();
        execution.provider_endpoint_id = saved.provider_endpoint_id.clone();
        execution.model = saved.model.clone();
        if !replace_workspace {
            execution.source_root_fingerprint = saved.source_root_fingerprint.clone();
        }
    }
    let WorkflowAutomationTrigger::Schedule { ref cron } = prepared.input.trigger else {
        unreachable!()
    };
    let preview =
        preview_workflow_cron_schedule(cron, &prepared.schedule_config.timezone, Utc::now(), 5)?;
    if creating {
        if let Ok(previous) = ctx
            .db
            .get_workflow_automation(prepared.input.id.as_deref().unwrap())
        {
            if serde_json::to_value(as_input(&previous))? != serde_json::to_value(&prepared.input)?
                || previous.schedule_config != prepared.schedule_config
            {
                return Err(CoreError::Conflict("requestId already belongs to a different task definition; get it or choose a new requestId".into()));
            }
            return Ok(
                json!({"task":previous,"preview":preview,"created":false,"runtime":"Nexa must remain running"}),
            );
        }
    }
    let task = ctx.db.save_authored_workflow_automation(
        &prepared.input,
        &prepared.schedule_config,
        None,
        Some(if creating {
            0
        } else {
            args.expected_revision.unwrap()
        }),
    )?;
    Ok(
        json!({"task":task,"preview":preview,"created":creating,"runtime":"Nexa must remain running"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{conversation::CreateConversationInput, db::Database};

    fn conversation(db: &Database, project_id: Option<String>) -> String {
        db.create_conversation(&CreateConversationInput {
            provider: "mock".into(),
            model: "mock".into(),
            system_prompt: None,
            collection_context: None,
            project_id,
            persona_id: None,
        })
        .unwrap()
        .id
    }
    async fn call(db: &Database, conversation: &str, args: Value) -> Result<Value, CoreError> {
        let registry = super::super::default_tool_registry();
        let text = args.to_string();
        let mut context = ToolExecutionContext::new("automation-test", &text, db, &[])
            .with_conversation_id(Some(conversation));
        context.tool_registry = Some(&registry);
        AutomationTool
            .execute(context)
            .await
            .map(|result| serde_json::from_str(&result.content).unwrap())
    }
    fn create() -> Value {
        json!({"action":"create","requestId":"morning-review","name":"Morning review","prompt":"Review today's priorities.","cron":"0 9 * * 1-5","timezone":"Asia/Shanghai"})
    }
    #[tokio::test]
    async fn chat_automation_create_retry_preview_and_history_share_durable_records() {
        let db = Database::open_memory().unwrap();
        let conversation = conversation(&db, None);
        let first = call(&db, &conversation, create()).await.unwrap();
        let retry = call(&db, &conversation, create()).await.unwrap();
        assert_eq!(first["task"]["id"], retry["task"]["id"]);
        assert_eq!(retry["created"], false);
        assert_eq!(db.list_workflow_automations().unwrap().len(), 1);
        assert_eq!(first["task"]["definitionRevision"], 1);
        assert_eq!(first["task"]["approvalPolicy"]["allowedTools"], json!([]));
        assert_eq!(first["preview"]["occurrences"].as_array().unwrap().len(), 5);
        let id = first["task"]["id"].as_str().unwrap();
        assert!(db
            .get_workflow_automation(id)
            .unwrap()
            .next_run_at
            .is_some());
        assert_eq!(
            call(&db, &conversation, json!({"action":"history","id":id}))
                .await
                .unwrap()["runs"],
            json!([])
        );
        let mut changed = create();
        changed["prompt"] = json!("A different task");
        assert!(call(&db, &conversation, changed).await.is_err());
    }
    #[tokio::test]
    async fn chat_automation_updates_preserve_policy_and_reject_stale_writes() {
        let db = Database::open_memory().unwrap();
        let conversation = conversation(&db, None);
        let first = call(&db, &conversation, create()).await.unwrap();
        let id = &first["task"]["id"];
        let edited = call(
            &db,
            &conversation,
            json!({"action":"update","id":id,"expectedRevision":1,"name":"New name"}),
        )
        .await
        .unwrap();
        assert_eq!(edited["task"]["prompt"], first["task"]["prompt"]);
        assert_eq!(
            edited["task"]["scheduleConfig"],
            first["task"]["scheduleConfig"]
        );
        assert!(call(
            &db,
            &conversation,
            json!({"action":"delete","id":id,"expectedRevision":1})
        )
        .await
        .is_err());
        assert!(db
            .delete_workflow_automation_at_revision(id.as_str().unwrap(), 1)
            .is_err());
        let paused = call(
            &db,
            &conversation,
            json!({"action":"pause","id":id,"expectedRevision":2}),
        )
        .await
        .unwrap();
        assert_eq!(paused["task"]["enabled"], false);
        assert!(paused["task"]["nextRunAt"].is_null());
        let resumed = call(
            &db,
            &conversation,
            json!({"action":"resume","id":id,"expectedRevision":3}),
        )
        .await
        .unwrap();
        assert_eq!(resumed["task"]["enabled"], true);
        call(
            &db,
            &conversation,
            json!({"action":"delete","id":id,"expectedRevision":4}),
        )
        .await
        .unwrap();
        assert!(db.list_workflow_automations().unwrap().is_empty());
    }
    #[tokio::test]
    async fn chat_automation_rejects_invalid_cron_timezone_sources_and_recursive_tools() {
        let db = Database::open_memory().unwrap();
        let conversation = conversation(&db, None);
        for (field, value) in [
            ("cron", json!("* * * * * *")),
            ("timezone", json!("Mars/Base")),
            ("sourceIds", json!(["outside-scope"])),
            ("allowedTools", json!(["automation"])),
            ("allowedTools", json!(["unavailable_tool"])),
        ] {
            let mut request = create();
            request[field] = value;
            assert!(
                call(&db, &conversation, request).await.is_err(),
                "accepted {field}"
            );
        }
        assert!(db.list_workflow_automations().unwrap().is_empty());
        let mut request = create();
        request["allowedTools"] = json!(["fetch_url"]);
        let saved = call(&db, &conversation, request).await.unwrap();
        assert_eq!(
            call(
                &db,
                &conversation,
                json!({"action":"get","id":saved["task"]["id"]})
            )
            .await
            .unwrap()["id"],
            saved["task"]["id"]
        );
    }
    #[tokio::test]
    async fn chat_automation_cannot_read_or_mutate_another_project() {
        let db = Database::open_memory().unwrap();
        let global = conversation(&db, None);
        let saved = call(&db, &global, create()).await.unwrap();
        let project = db
            .create_project(&crate::project::CreateProjectInput {
                name: "Other".into(),
                description: None,
                workspace_roots: None,
                icon: None,
                color: None,
                system_prompt: None,
                source_scope: None,
            })
            .unwrap();
        let other = conversation(&db, Some(project.id));
        assert_eq!(
            call(&db, &other, json!({"action":"list"})).await.unwrap()["tasks"],
            json!([])
        );
        assert!(call(
            &db,
            &other,
            json!({"action":"get","id":saved["task"]["id"]})
        )
        .await
        .is_err());
        assert!(call(
            &db,
            &other,
            json!({"action":"delete","id":saved["task"]["id"],"expectedRevision":1})
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn chat_automation_is_discoverable_and_mutations_are_permission_gated() {
        let db = Database::open_memory().unwrap();
        let preview = call(
            &db,
            "",
            json!({"action":"preview","cron":"0 9 * * *","timezone":"UTC","limit":25}),
        )
        .await
        .unwrap();
        assert_eq!(preview["occurrences"].as_array().unwrap().len(), 20);
        let registry = super::super::default_tool_registry();
        let args = r#"{"query":"cron 定时任务"}"#;
        let mut ctx = ToolExecutionContext::new("search", args, &db, &[]);
        ctx.tool_registry = Some(&registry);
        let found = super::super::tool_search_tool::ToolSearchTool
            .execute(ctx)
            .await
            .unwrap();
        assert!(found.content.contains("automation"));
        assert!(AutomationTool.access_profile(&create()).needs_approval);
        assert!(
            !AutomationTool
                .access_profile(&json!({"action":"list"}))
                .needs_approval
        );
        assert_eq!(
            super::super::capability::scheduled_workspace_tool_class("automation"),
            super::super::capability::ScheduledWorkspaceToolClass::Unsupported
        );
        assert!(
            validate_action_fields(r#"{"action":"pause","id":"x","prompt":"hidden edit"}"#)
                .is_err()
        );
    }

    #[tokio::test]
    async fn chat_automation_rechecks_prepared_project_scope() {
        let db = Database::open_memory().unwrap();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let add = |path: &std::path::Path| {
            db.add_source(crate::sources::CreateSourceInput {
                root_path: path.to_string_lossy().into(),
                include_globs: vec![],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .unwrap()
            .id
        };
        let source_a = add(a.path());
        let source_b = add(b.path());
        let project = db
            .create_project(&crate::project::CreateProjectInput {
                name: "Scoped".into(),
                description: None,
                workspace_roots: None,
                icon: None,
                color: None,
                system_prompt: None,
                source_scope: Some(vec![source_a.clone(), source_b]),
            })
            .unwrap();
        let conversation = conversation(&db, Some(project.id));
        let registry = super::super::default_tool_registry();
        let mut args = create();
        args["sourceIds"] = json!([]);
        args["allowedTools"] = json!(["fetch_url"]);
        let scope = vec![source_a.clone()];
        let arguments = args.to_string();
        let mut context = ToolExecutionContext::new("scoped", &arguments, &db, &scope)
            .with_conversation_id(Some(&conversation));
        context.tool_registry = Some(&registry);
        assert!(AutomationTool
            .execute(context)
            .await
            .unwrap_err()
            .to_string()
            .contains("exceed"));
        assert!(db.list_workflow_automations().unwrap().is_empty());
        args.as_object_mut().unwrap().remove("sourceIds");
        let arguments = args.to_string();
        let mut context = ToolExecutionContext::new("scoped", &arguments, &db, &scope)
            .with_conversation_id(Some(&conversation));
        context.tool_registry = Some(&registry);
        AutomationTool.execute(context).await.unwrap();
        assert_eq!(
            db.list_workflow_automations().unwrap()[0].source_scope,
            vec![source_a]
        );
    }

    #[tokio::test]
    async fn chat_automation_preserves_pinned_route_on_rename_and_resume() {
        let db = Database::open_memory().unwrap();
        let conversation = conversation(&db, None);
        let input: crate::conversation::SaveAgentConfigInput = serde_json::from_value(json!({
            "id":"pinned", "name":"Pinned", "provider":"open_ai", "apiKey":"fixture-only", "model":"gpt-6.1-sol", "isDefault":true,
            "baseUrl":"https://api.openai.com/v1", "providerEndpointId":"text:openai"
        })).unwrap();
        let agent = db.save_agent_config(&input).unwrap();
        let saved = call(&db, &conversation, create()).await.unwrap();
        let id = saved["task"]["id"].as_str().unwrap();
        let task = db.get_workflow_automation(id).unwrap();
        let mut config = task.schedule_config.clone();
        config.execution_policy.agent_config_id = Some(agent.id);
        config.execution_policy.provider = Some("open_ai".into());
        config.execution_policy.provider_endpoint_id = Some("text:openai".into());
        config.execution_policy.model = Some(input.model.clone());
        db.save_authored_workflow_automation(&as_input(&task), &config, None, Some(1))
            .unwrap();
        let mut changed = input.clone();
        changed.provider = "anthropic".into();
        changed.provider_endpoint_id = Some("text:anthropic".into());
        changed.base_url = Some("https://api.anthropic.com/v1".into());
        changed.model = "claude-sonnet-5-5".into();
        db.save_agent_config(&changed).unwrap();
        let renamed = call(
            &db,
            &conversation,
            json!({"action":"update","id":id,"expectedRevision":2,"name":"Only rename"}),
        )
        .await
        .unwrap();
        assert_eq!(
            renamed["task"]["scheduleConfig"],
            serde_json::to_value(&config).unwrap()
        );
        call(
            &db,
            &conversation,
            json!({"action":"pause","id":id,"expectedRevision":3}),
        )
        .await
        .unwrap();
        let resumed = call(
            &db,
            &conversation,
            json!({"action":"resume","id":id,"expectedRevision":4}),
        )
        .await
        .unwrap();
        assert_eq!(
            resumed["task"]["scheduleConfig"],
            serde_json::to_value(&config).unwrap()
        );
    }
}
