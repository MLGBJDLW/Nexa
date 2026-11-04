//! Phone host over the shared executor, run ledger, approvals and file tools.
use crate::{connections, phone_tool::PhoneTool, platform};
use base64::Engine;
use nexa_core::{
    agent::{AgentConfig, AgentExecutionMode, CancellationToken},
    agent_run::AgentRunEvent,
    agent_session::ExecutorAgentSession,
    approval::{ApprovalDecision, ApprovalRequest, ToolApprovalMode},
    conversation::{
        conversation_message_is_model_history, conversation_message_to_llm, AgentTaskRun,
        ConversationMessage, CreateConversationInput, ImageAttachment, TurnModelSelection,
    },
    db::Database,
    db_executor::DatabaseExecutor,
    llm::{create_provider, Role},
    run_event_outbox::{AgentRunEventDelivery, AgentRunEventOutboxes},
    runtime::{
        AgentSession, AgentSessionConfig, AgentTurnAttachment, AgentTurnInput, RuntimeHostSurface,
    },
    tools::{
        create_file_tool::CreateFileTool, edit_file_tool::EditFileTool,
        fetch_url_tool::FetchUrlTool, file_tool::FileTool, list_dir_tool::ListDirTool,
        web_search_tool::WebSearchTool, ToolRegistry,
    },
    workspace::Workspace,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tauri::{AppHandle, Emitter};
use tokio::sync::{oneshot, Mutex};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSettings {
    #[serde(default)]
    pub connection_id: Option<String>,
    pub provider_type: String,
    pub base_url: Option<String>,
    pub model: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnOptions {
    pub connection_id: Option<String>,
    pub model_selection: Option<TurnModelSelection>,
    #[serde(default)]
    pub attachments: Vec<ImageAttachment>,
    #[serde(default)]
    pub execution_mode: AgentExecutionMode,
    #[serde(default)]
    pub tool_approval_mode: ToolApprovalMode,
}

impl TurnOptions {
    fn validate_attachments(&self) -> Result<(), String> {
        if self.attachments.len() > 8
            || self
                .attachments
                .iter()
                .map(|a| a.base64_data.len())
                .sum::<usize>()
                > 11_200_000
        {
            return Err("Attachments exceed 8 images or 8 MiB in total.".into());
        }
        let mut bytes = 0;
        for attachment in &self.attachments {
            if !matches!(
                attachment.media_type.as_str(),
                "image/png" | "image/jpeg" | "image/webp" | "image/gif"
            ) {
                return Err(
                    "Phone attachments currently support PNG, JPEG, WebP and GIF images.".into(),
                );
            }
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&attachment.base64_data)
                .map_err(|_| "Invalid image encoding.")?;
            if decoded.is_empty() {
                return Err("The image is empty.".into());
            }
            bytes += decoded.len();
        }
        if bytes > 8 * 1024 * 1024 {
            return Err("Attachments exceed 8 MiB in total.".into());
        }
        Ok(())
    }
}

struct Delivery(AppHandle);
impl AgentRunEventDelivery for Delivery {
    fn deliver_run_event(&self, conversation: &str, event: &AgentRunEvent) {
        let _ = self.0.emit(
            "mobile:run",
            json!({"conversationId":conversation,"event":event}),
        );
    }
    fn deliver_task_run_snapshot(&self, conversation: &str, run: AgentTaskRun) {
        let _ = self.0.emit(
            "mobile:snapshot",
            json!({"conversationId":conversation,"run":run}),
        );
    }
}

pub struct LocalRuntime {
    pub db: Database,
    workspace: Workspace,
    outboxes: AgentRunEventOutboxes,
    // Admission spans idempotent persistence and spawning. One phone task at a
    // time avoids competing cross-app actions and keeps Stop unambiguous.
    active: Mutex<Option<(String, CancellationToken)>>,
    pending: Mutex<HashMap<String, (ApprovalRequest, oneshot::Sender<ApprovalDecision>)>>,
}

impl LocalRuntime {
    pub fn workspace_dir(&self) -> PathBuf {
        PathBuf::from(&self.workspace.roots[0])
    }
    pub async fn open(app: &AppHandle, directory: PathBuf) -> Result<Arc<Self>, String> {
        std::fs::create_dir_all(directory.join("workspace")).map_err(err)?;
        let db = Database::new(directory.join("nexa-mobile.db")).map_err(err)?;
        let outboxes = AgentRunEventOutboxes::new(
            DatabaseExecutor::new(db.clone(), 64).map_err(err)?,
            Arc::new(Delivery(app.clone())),
        );
        // Never replay a possibly completed phone action after process death.
        outboxes
            .recover_after_restart_with_checkpoints()
            .await
            .map_err(err)?;
        Ok(Arc::new(Self {
            db,
            workspace: Workspace::validate(&[directory
                .join("workspace")
                .to_string_lossy()
                .into_owned()])
            .map_err(err)?,
            outboxes,
            active: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
        }))
    }

    pub async fn approvals(&self, conversation_id: &str) -> Result<Value, String> {
        let active = self.active.lock().await;
        if active.as_ref().is_none_or(|(id, _)| id != conversation_id) {
            return Ok(json!([]));
        }
        let run = self.db.remote_latest_run(conversation_id).map_err(err)?;
        let run_id = run.as_deref();
        let approvals: Vec<_> = self
            .pending
            .lock()
            .await
            .values()
            .map(|(request, _)| json!({"runId":run_id,"request":request}))
            .collect();
        Ok(json!(approvals))
    }

    pub async fn start(
        self: &Arc<Self>,
        app: AppHandle,
        conversation_id: Option<String>,
        mut text: String,
        request_id: String,
        checkpoint_id: Option<String>,
        options: TurnOptions,
    ) -> Result<Value, String> {
        if let Some(id) = &checkpoint_id {
            let checkpoint = self.db.get_task_resume_checkpoint(id).map_err(err)?;
            let run = self
                .db
                .get_agent_task_run(&checkpoint.run_id)
                .map_err(err)?;
            if conversation_id.as_deref() != Some(&run.conversation_id) {
                return Err("Checkpoint belongs to another conversation.".into());
            }
            text = checkpoint.resume_prompt;
        }
        options.validate_attachments()?;
        if text.trim().is_empty() && options.attachments.is_empty() {
            return Err("Enter a message first.".into());
        }
        uuid::Uuid::parse_str(&request_id)
            .map_err(|_| "A stable request ID is required.".to_string())?;
        let mut active = self.active.lock().await;
        let mut saved = connections::get(&app, options.connection_id.as_deref()).await?;
        if let Some(selection) = &options.model_selection {
            selection.apply_to_shared(&mut saved).map_err(err)?;
        }
        if nexa_core::provider_catalog::retired_model_for_endpoint(
            &saved.provider,
            saved.base_url.as_deref(),
            &saved.model,
        )
        .is_some()
        {
            return Err("This model is no longer available on this provider. Choose a supported model from the model picker.".into());
        }
        let settings = crate::connections::settings(&saved);
        let provider_type =
            nexa_core::provider_registry::provider_type_from_key(&settings.provider_type)
                .ok_or("Unsupported provider type.")?;
        let provider = create_provider(connections::provider_config(&saved)?).map_err(err)?;
        let conversation = if let Some(id) = conversation_id {
            self.db.get_conversation(&id).map_err(err)?
        } else {
            // The UI creates and retains a conversation before sending so a
            // transport retry always addresses the same idempotency scope.
            return Err("Create a conversation before sending.".into());
        };
        let previous = self
            .db
            .find_agent_turn_by_idempotency_key(&conversation.id, &request_id)
            .map_err(err)?;
        if active.is_some() && previous.is_none() {
            return Err("Stop or finish the current phone task first.".into());
        }
        let history = self
            .db
            .get_messages(&conversation.id)
            .map_err(err)?
            .iter()
            .filter(|m| conversation_message_is_model_history(m))
            .map(conversation_message_to_llm)
            .collect();
        let history = nexa_core::llm::message_validation::repair_persisted_message_history(
            history,
            Some(&conversation.id),
        )
        .messages;
        let message = ConversationMessage {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation.id.clone(),
            role: Role::User,
            content: text.clone(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            artifacts: Some(
                json!({"phoneTurnOptions":{"connectionId":saved.id,"modelSelection":options.model_selection,"executionMode":options.execution_mode,"toolApprovalMode":options.tool_approval_mode}}),
            ),
            token_count: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            sort_order: self
                .db
                .next_message_sort_order(&conversation.id)
                .map_err(err)?,
            thinking: None,
            image_attachments: (!options.attachments.is_empty())
                .then(|| options.attachments.clone()),
        };
        if previous.is_some() {
            // Re-enter the authoritative payload check, rather than merely
            // returning an old ID for a different message or provider.
            return serde_json::to_value(
                self.db
                    .create_agent_turn_and_run(
                        &message,
                        &text,
                        Some(&settings.provider_type),
                        Some(&settings.model),
                        &request_id,
                    )
                    .map_err(err)?,
            )
            .map_err(err);
        }
        let token = CancellationToken::new();
        let weak = Arc::downgrade(self);
        let stopped = token.clone();
        let channel = tauri::ipc::Channel::<Value>::new(move |_| {
            stopped.cancel();
            if let Some(runtime) = weak.upgrade() {
                tauri::async_runtime::spawn(async move {
                    runtime.pending.lock().await.clear();
                });
            }
            Ok(())
        });
        platform::call(
            &app,
            "registerListener",
            json!({"event":"taskStop","handler":channel}),
        )
        .await?;
        if let Err(error) = platform::call(&app, "taskService", json!({"active":true})).await {
            let _ = platform::call(
                &app,
                "removeListener",
                json!({"event":"taskStop","channelId":channel.id()}),
            )
            .await;
            return Err(error);
        }
        let persisted = if let Some(checkpoint) = &checkpoint_id {
            self.db.resume_agent_turn_from_checkpoint(
                &message,
                Some(&settings.provider_type),
                Some(&settings.model),
                &request_id,
                checkpoint,
            )
        } else {
            self.db.create_agent_turn_and_run(
                &message,
                &text,
                Some(&settings.provider_type),
                Some(&settings.model),
                &request_id,
            )
        };
        let launch = match persisted {
            Ok(launch) => launch,
            Err(error) => {
                let _ = platform::call(&app, "taskService", json!({"active":false})).await;
                let _ = platform::call(
                    &app,
                    "removeListener",
                    json!({"event":"taskStop","channelId":channel.id()}),
                )
                .await;
                return Err(err(error));
            }
        };
        if launch.reused {
            let _ = platform::call(&app, "taskService", json!({"active":false})).await;
            let _ = platform::call(
                &app,
                "removeListener",
                json!({"event":"taskStop","channelId":channel.id()}),
            )
            .await;
            return serde_json::to_value(launch).map_err(err);
        }
        if message.sort_order == 0 {
            let _ = self.db.update_conversation_title(
                &conversation.id,
                &nexa_core::conversation::fallback_title(&text),
            );
        }
        *active = Some((conversation.id.clone(), token.clone()));
        let mut tools = ToolRegistry::new().with_workspace(Some(self.workspace.clone()));
        tools.register(Box::new(FileTool));
        tools.register(Box::new(ListDirTool));
        tools.register(Box::new(CreateFileTool));
        tools.register(Box::new(EditFileTool));
        tools.register(Box::new(PhoneTool(app.clone())));
        tools.register(Box::new(WebSearchTool));
        tools.register(Box::new(FetchUrlTool));
        let contract = nexa_core::llm::model_contract::resolve_configured_model_contract(
            provider_type,
            saved.base_url.as_deref(),
            &saved.model,
        );
        let context_window = saved.context_window.and_then(|value| value.try_into().ok());
        let mut config = AgentConfig {
            model: Some(settings.model.clone()),
            provider_type: Some(provider_type),
            temperature: saved.temperature.map(|value| value as f32),
            context_window,
            context_window_resolution: Some(contract.context_window(context_window)),
            catalog_limits_authoritative: Some(contract.catalog_authoritative),
            resolved_catalog_limits: contract.catalog_limits(),
            reasoning_enabled: saved.reasoning_enabled,
            thinking_budget: saved
                .thinking_budget
                .and_then(|value| value.try_into().ok()),
            reasoning_effort: saved
                .reasoning_effort
                .as_deref()
                .and_then(nexa_core::llm::ReasoningEffort::from_wire),
            require_tool_confirmation: true,
            tool_approval_mode: options.tool_approval_mode,
            execution_mode: options.execution_mode,
            ..Default::default()
        };
        config.volatile_system_sections.push(format!("You run on the user's Android phone. Files are confined to {}. Only advertise the registered tools. phone_control observes Android accessibility UI and acts only on a fresh observation. The user must explicitly enable accessibility. After any phone action observe and verify its result. Do not infer success from dispatch alone. Ask in chat when more user input is needed.", self.workspace.roots[0]));
        config
            .volatile_system_sections
            .push(options.tool_approval_mode.prompt_guidance().into());
        let runtime = self.clone();
        let approval_runtime = self.clone();
        let confirmation_runtime = self.clone();
        let approve_app = app.clone();
        let confirm_app = app.clone();
        let mut session = ExecutorAgentSession::new(
            AgentSessionConfig {
                session_id: conversation.id.clone(),
                conversation_id: Some(conversation.id.clone()),
                task_run_id: Some(launch.run_id.clone()),
                host_surface: RuntimeHostSurface::Mobile,
                model: Some(settings.model),
                ..Default::default()
            },
            provider,
            tools,
            config,
            self.db.clone(),
        )
        .with_history(history, message.sort_order + 1)
        .with_skills_override(Vec::new())
        .with_auto_loaded_skills_override(Vec::new())
        .with_event_outboxes(self.outboxes.clone())
        .with_cancel_token(token.clone())
        .with_approval_callback(Arc::new(move |request| {
            let runtime = approval_runtime.clone();
            let app = approve_app.clone();
            Box::pin(async move { runtime.approve(&app, request).await })
        }))
        .with_confirmation_callback(Arc::new(move |message| {
            let runtime = confirmation_runtime.clone();
            let app = confirm_app.clone();
            Box::pin(async move {
                let request = ApprovalRequest::new(
                    uuid::Uuid::new_v4().to_string(),
                    "phone_task",
                    &json!({}),
                    nexa_core::approval::ApprovalRisk::High,
                    message,
                );
                runtime.approve(&app, request).await == ApprovalDecision::AllowOnce
            })
        }));
        let launch_result = serde_json::to_value(&launch).map_err(err)?;
        let mut turn_input = AgentTurnInput::text(text);
        turn_input.mode = options.execution_mode;
        turn_input.attachments = options
            .attachments
            .into_iter()
            .map(|image| AgentTurnAttachment {
                id: image
                    .attachment_id
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                kind: Some("image".into()),
                path: None,
                mime_type: Some(image.media_type),
                base64_data: Some(image.base64_data),
            })
            .collect();
        tokio::spawn(async move {
            let result = session.start_turn(turn_input).await;
            if let Err(error) = result {
                if let Ok(outbox) = runtime
                    .outboxes
                    .open(&conversation.id, &launch.run_id)
                    .await
                {
                    let _ = outbox.submit(AgentRunEvent::terminal_error(
                        &launch.run_id,
                        Some(&launch.turn_id),
                        0,
                        "Phone task failed",
                        "failed",
                        Some(&json!({"error":error.to_string()})),
                    ));
                    let _ = outbox.wait_for_terminal_commit().await;
                }
            }
            runtime.pending.lock().await.clear();
            let mut active = runtime.active.lock().await;
            let _ = platform::call(&app, "taskService", json!({"active":false})).await;
            let _ = platform::call(
                &app,
                "removeListener",
                json!({"event":"taskStop","channelId":channel.id()}),
            )
            .await;
            active.take();
            let _ = app.emit("mobile:refresh", ());
        });
        Ok(launch_result)
    }

    async fn approve(&self, app: &AppHandle, request: ApprovalRequest) -> ApprovalDecision {
        let (tx, rx) = oneshot::channel();
        let id = request.id.clone();
        let prompt = format!("{}\n{}", request.reason, request.arguments_preview);
        self.pending.lock().await.insert(id.clone(), (request, tx));
        let _ = app.emit("mobile:refresh", ());
        let result = tokio::select! {
            result = rx => result.unwrap_or(ApprovalDecision::Deny),
            result = platform::call(app,"approval",json!({"id":id,"reason":prompt})) => {
                if result.is_ok_and(|v|v["allow"] == true) { ApprovalDecision::AllowOnce } else { ApprovalDecision::Deny }
            }
        };
        let _ = platform::call(app, "approval", json!({"id":id,"cancel":true})).await;
        self.pending.lock().await.remove(&id);
        if result == ApprovalDecision::AllowOnce {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        result
    }

    pub async fn respond(&self, id: &str, allow: bool) -> Result<(), String> {
        let (_, sender) = self
            .pending
            .lock()
            .await
            .remove(id)
            .ok_or("This approval is no longer pending.")?;
        sender
            .send(if allow {
                ApprovalDecision::AllowOnce
            } else {
                ApprovalDecision::Deny
            })
            .map_err(|_| "This task has ended.".into())
    }

    pub async fn stop(&self) {
        if let Some((_, token)) = self.active.lock().await.as_ref() {
            token.cancel();
        }
        self.pending.lock().await.clear();
    }

    pub fn create(&self, settings: &LocalSettings) -> Result<Value, String> {
        serde_json::to_value(
            self.db
                .create_conversation(&CreateConversationInput {
                    provider: settings.provider_type.clone(),
                    model: settings.model.clone(),
                    system_prompt: None,
                    collection_context: None,
                    project_id: None,
                    persona_id: None,
                })
                .map_err(err)?,
        )
        .map_err(err)
    }
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
