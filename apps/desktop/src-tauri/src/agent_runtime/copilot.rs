use super::copilot_events::{EventLedger, EventStream};
use super::projection::Projection;
use super::*;
use github_copilot_sdk::session_events::ReasoningSummary;
use github_copilot_sdk::types::{
    DeferMode, ToolBinaryResult, ToolInvocation, ToolResult, ToolResultExpanded, ToolSearchConfig,
};
use github_copilot_sdk::{
    Attachment, Client, ClientMode, ClientOptions, MessageOptions, SessionConfig, SessionEvent,
    SystemMessageConfig, Tool, ToolSet,
};
use nexa_core::agent::StreamBlockChannel;
use nexa_core::llm::ToolCallRequest;
#[cfg(test)]
use std::collections::HashSet;
use std::collections::VecDeque;
use std::time::Duration;

struct ToolBridge {
    tools: Arc<ExternalToolSession>,
    fatal: mpsc::Sender<CoreError>,
}

fn sdk_error(error: github_copilot_sdk::Error) -> CoreError {
    use github_copilot_sdk::{ErrorKind, SessionErrorKind};
    if error.is_transport_failure()
        || matches!(
            error.kind(),
            ErrorKind::Session(SessionErrorKind::EventLoopClosed | SessionErrorKind::Timeout(_))
        )
    {
        CoreError::StreamIncomplete(format!("Copilot connection interrupted: {error}"))
    } else {
        protocol_error(error)
    }
}

fn client_options(binary: std::path::PathBuf) -> Result<ClientOptions, CoreError> {
    // Empty mode requires an explicit persistence owner. Use the same official
    // home as Copilot login; never copy credentials into Nexa or a temp home.
    let directory = std::env::var_os("COPILOT_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|home| std::path::PathBuf::from(home).join(".copilot"))
        })
        .filter(|path| path.is_absolute())
        .ok_or_else(|| protocol_error("Copilot home must be an absolute directory"))?;
    Ok(with_login_credential_backend(
        super::copilot_options::client_options(binary)
            .with_mode(ClientMode::Empty)
            .with_base_directory(directory),
        std::env::var_os("COPILOT_DISABLE_KEYTAR"),
    ))
}

fn with_login_credential_backend(
    mut options: ClientOptions,
    inherited: Option<std::ffi::OsString>,
) -> ClientOptions {
    // SDK 1.0.11 injects DISABLE_KEYTAR=1 in Empty mode, then applies caller
    // env/env_remove. Tool isolation remains Empty; authentication must use
    // precisely the same keychain setting as the ordinary login/account CLI.
    // Neither path extracts or copies a token into Nexa.
    match inherited {
        Some(value) => {
            options
                .env
                .push((std::ffi::OsString::from("COPILOT_DISABLE_KEYTAR"), value));
            options
        }
        None => options.with_env_remove(["COPILOT_DISABLE_KEYTAR"]),
    }
}

#[async_trait::async_trait]
impl github_copilot_sdk::tool::ToolHandler for ToolBridge {
    async fn call(
        &self,
        invocation: ToolInvocation,
    ) -> Result<ToolResult, github_copilot_sdk::Error> {
        let result = self
            .tools
            .execute(ToolCallRequest {
                id: invocation.tool_call_id,
                name: invocation.tool_name,
                arguments: invocation.arguments.to_string(),
                thought_signature: None,
            })
            .await;
        Ok(match result {
            Ok(output) => {
                let mut result = ToolResultExpanded::new(
                    output.result.content,
                    if output.result.is_error {
                        "failure"
                    } else {
                        "success"
                    },
                );
                let mut images = Vec::new();
                for part in output.visual_parts {
                    match part {
                        ContentPart::Image { media_type, data } => images.push(ToolBinaryResult {
                            data,
                            mime_type: media_type,
                            r#type: "image".into(),
                            description: Some("Current Nexa tool observation".into()),
                        }),
                        ContentPart::Document { document } => {
                            result.text_result_for_llm.push_str(
                                &document.fallback("Copilot document protocol unverified"),
                            );
                        }
                        ContentPart::Text { text } => {
                            result.text_result_for_llm.push('\n');
                            result.text_result_for_llm.push_str(&text);
                        }
                        _ => {}
                    }
                }
                if !images.is_empty() {
                    result.binary_results_for_llm = Some(images);
                }
                ToolResult::Expanded(result)
            }
            Err(error) => {
                let message = error.to_string();
                let _ = self.fatal.try_send(error);
                ToolResult::Expanded(ToolResultExpanded::new(message, "failure"))
            }
        })
    }
}

pub(super) async fn run(request: AgentRuntimeTurnRequest) -> Result<Message, CoreError> {
    let cancellation = request.cancellation.clone();
    let model_id = request
        .config
        .model
        .clone()
        .ok_or_else(|| protocol_error("select a Copilot model first"))?;
    let connect = async {
        let binary = tokio::task::spawn_blocking(
            crate::commands::subscription_accounts::resolve_copilot_binary,
        )
        .await
        .map_err(protocol_error)?
        .map_err(protocol_error)?;
        let client = Client::start(client_options(binary)?)
            .await
            .map_err(sdk_error)?;
        let models = client.list_models().await.map_err(sdk_error)?;
        let model = models.into_iter().find(|model| model.id == model_id).ok_or_else(|| protocol_error("the selected model is not available to this Copilot account; refresh its model list"))?;
        Ok::<_, CoreError>((client, model))
    };
    let (client, model) = tokio::select! {
        _ = cancellation.cancelled() => return Err(CoreError::Cancelled("Stopped during Copilot connection".into())),
        result = tokio::time::timeout(Duration::from_secs(45), connect) => result.map_err(|_| CoreError::TransientLlm("Copilot connection timed out".into()))??,
    };
    let native_vision = model
        .capabilities
        .supports
        .as_ref()
        .and_then(|supports| supports.vision)
        .unwrap_or(false);
    let context_policy = request
        .db
        .load_model_context_policy("github_copilot", None, &model_id)?
        .unwrap_or_default();
    let context_tier = context_policy.context_tier.as_deref().unwrap_or("default");
    if !crate::commands::subscription_accounts::copilot_context_tiers(&model)
        .iter()
        .any(|tier| tier.id == context_tier)
    {
        return Err(protocol_error("The saved context tier is not available for this Copilot model. Open Context policy and select an available tier."));
    }
    let mut turn = request.prepare(native_vision)?;
    let (fatal_tx, mut fatal_rx) = mpsc::channel(1);
    let bridge = Arc::new(ToolBridge {
        tools: turn.transcript.nexa_tools().clone(),
        fatal: fatal_tx,
    });
    let tools = turn
        .transcript
        .nexa_tools()
        .definitions()
        .into_iter()
        // Copilot owns discovery/loading for its loop. Its native search sees
        // exactly this enabled custom catalog; do not expose a second search
        // tool that cannot activate definitions inside the upstream runtime.
        .filter(|definition| definition.name != "tool_search")
        .map(|definition| {
            Tool::new(definition.name)
                .with_description(definition.description)
                .with_parameters(definition.parameters)
                // Register every callback without charging every schema to each
                // request. Discovery is explicitly enabled below in Empty mode.
                .with_defer(DeferMode::Auto)
                // Deferred discovery can otherwise resolve names such as
                // read_file to the CLI's built-in handler after loading our
                // schema. Keep every advertised Nexa action on its callback.
                .with_overrides_built_in_tool(true)
                // Only this custom callback skips CLI permission prompts. Nexa's
                // shared dispatcher performs the actual policy and user approval.
                .with_skip_permission(true)
                .with_handler(bridge.clone())
        })
        .collect::<Vec<_>>();
    let has_tools = !tools.is_empty();
    let mut available_tools = ToolSet::new().add_custom("*").map_err(protocol_error)?;
    let mut system_prompt = turn.system_prompt.clone();
    if has_tools {
        available_tools = available_tools
            .add_builtin("tool_search_tool")
            .map_err(protocol_error)?;
        system_prompt.push_str("\nFor this Copilot runtime, use tool_search_tool to discover and load enabled Nexa tools. It is the tool_search capability mentioned in these instructions. All discovered actions still execute through Nexa's tool dispatcher.");
    }
    let mut config = SessionConfig::default()
        .with_client_name("Nexa")
        .with_model(&model_id)
        .with_context_tier(context_tier)
        .with_streaming(true)
        .with_reasoning_summary(ReasoningSummary::Concise)
        .with_tools(tools)
        .with_tool_search(
            ToolSearchConfig::new()
                .with_enabled(has_tools)
                .with_defer_threshold(1),
        )
        .with_available_tools(available_tools.to_vec())
        .with_system_message(
            SystemMessageConfig::new()
                .with_mode("replace")
                .with_content(system_prompt),
        )
        .deny_all_permissions();
    if let Some(effort) = turn.config.reasoning_effort.as_ref() {
        let effort = serde_json::to_value(effort)
            .map_err(protocol_error)?
            .as_str()
            .ok_or_else(|| protocol_error("invalid reasoning effort"))?
            .to_string();
        if !crate::commands::subscription_accounts::copilot_reasoning_efforts(&model)
            .contains(&effort)
        {
            return Err(protocol_error(
                "the selected Copilot model does not support this reasoning effort",
            ));
        }
        config = config.with_reasoning_effort(effort);
    }
    let session = tokio::select! {
        _ = turn.cancellation.cancelled() => return Err(CoreError::Cancelled("Stopped during Copilot session creation".into())),
        result = tokio::time::timeout(Duration::from_secs(30), client.create_session(config)) => result.map_err(|_| CoreError::TransientLlm("Copilot session creation timed out".into()))?.map_err(sdk_error)?,
    };
    // Empty-mode sessions do not inherit the interactive CLI's presentation
    // settings. Summary verbosity and emitting readable summary events are
    // separate options; effort alone does not enable Gemini/Claude thoughts.
    tokio::select! {
        _ = turn.cancellation.cancelled() => return Err(CoreError::Cancelled("Stopped during Copilot configuration".into())),
        result = tokio::time::timeout(Duration::from_secs(30), async { session.rpc().options().update(
            github_copilot_sdk::rpc::SessionUpdateOptionsParams {
                enable_reasoning_summaries: Some(true),
                ..Default::default()
            }
        ).await }) => { result.map_err(|_| CoreError::TransientLlm("Copilot reasoning configuration timed out".into()))?.map_err(sdk_error)?; },
    }
    let mut events = EventStream::start(session.subscribe())?;
    let initial = MessageOptions::new(&turn.prompt).with_attachments(
        turn.images
            .iter()
            .map(|(mime_type, data)| Attachment::Blob {
                data: data.clone(),
                mime_type: mime_type.clone(),
                display_name: None,
            })
            .collect(),
    );
    let mut projection = Projection::for_turn(&turn);
    let mut seen = EventLedger::default();
    let mut response = super::copilot_response::Response::default();
    let mut steering = VecDeque::new();
    let mut steering_closed = false;
    let mut completion_repairs = super::CompletionRepairState::default();
    let expects_reasoning = turn.config.reasoning_enabled != Some(false)
        && !crate::commands::subscription_accounts::copilot_reasoning_efforts(&model).is_empty();
    let run = async {
        let mut next = initial;
        loop {
            // The SDK's reliable waiter observes idle/error before the bounded
            // subscriber queue. Ephemeral idle cannot be recovered by replaying
            // persisted messages after lag. The host cancellation token owns
            // deadlines; do not accidentally adopt the SDK's 60-second default.
            let operation = session.send_and_wait(next.with_wait_timeout(Duration::from_secs(u32::MAX as u64)));
            tokio::pin!(operation);
            let mut completed = None;
            let mut idle_observed = false;
            loop {
                if completed.is_some() && idle_observed { break; }
                tokio::select! {
                    biased;
                    _ = turn.cancellation.cancelled() => return Err(CoreError::Cancelled("Stopped by user".into())),
                    error = fatal_rx.recv() => if let Some(error) = error { return Err(error); },
                    message = turn.steering.recv(), if !steering_closed => match message {
                        Some(message) => {
                            if steering.len() >= 64 || message.content.len() > 256 * 1024 { return Err(protocol_error("Copilot steering input budget exceeded")); }
                            steering.push_back(message);
                        },
                        None => steering_closed = true,
                    },
                    result = &mut operation, if completed.is_none() => {
                        if result.is_err() { completed = Some(result); break; }
                        completed = Some(result);
                    },
                    event = events.recv() => {
                        let event = event?;
                        if event.agent_id.as_deref().is_none_or(str::is_empty) {
                            if event.event_type == "session.idle" {
                                if event.data["aborted"] == true { return Err(CoreError::Cancelled("Copilot turn was aborted".into())); }
                                idle_observed = true;
                            } else if matches!(event.event_type.as_str(), "assistant.turn_start" | "tool.execution_start") { idle_observed = false; }
                        }
                        project_event(&mut projection, &mut response, &turn.events, &mut seen, &event).await?;
                        projection.persist_settled(&turn, response.take_settled()).await?;
                    }
                }
            };
            // Full durable records repair any dropped deltas, and the cursor
            // prevents re-counting or resurrecting already-checkpointed output.
            let history = tokio::select! {
                _ = turn.cancellation.cancelled() => return Err(CoreError::Cancelled("Stopped during Copilot history reconciliation".into())),
                result = tokio::time::timeout(Duration::from_secs(15), session.get_events()) => result,
            };
            if let Ok(Ok(history)) = history {
                // Native compaction can retire the upstream cursor. Never
                // replay that unanchored history or fail a complete live turn.
                for event in seen.backfill(history).unwrap_or_default() {
                    project_event(&mut projection, &mut response, &turn.events, &mut seen, &event).await?;
                    projection.persist_settled(&turn, response.take_settled()).await?;
                }
            } else {
                // The ordered spool already drained through native idle. An
                // optional history read cannot hang or invalidate that answer;
                // missing chunk groups still fail ensure_complete below.
                log::warn!("Copilot history reconciliation unavailable; retaining ordered live events");
            }
            if let Some(event) = completed.expect("native completion observed").map_err(sdk_error)? {
                project_event(&mut projection, &mut response, &turn.events, &mut seen, &event).await?;
                projection.persist_settled(&turn, response.take_settled()).await?;
            }
            response.ensure_complete()?;
            if let Some((guidance, milestones)) = turn.transcript.completion_repair_status().await {
                let guidance = completion_repairs.next(guidance, milestones)?;
                projection.defer_answer_for_verification(&turn).await?;
                next = MessageOptions::new(format!("Nexa host verification remains pending. {guidance}"));
                continue;
            }
            let Some(message) = steering.pop_front() else { return Ok(()); };
            if message.recovery_control.is_some() { return Err(protocol_error("Copilot manages its own recovery. Stop and start a new turn to change reasoning.")); }
            let attachments = message.parts.iter().filter_map(|part| match part { ContentPart::Image {media_type,data} => Some(Attachment::Blob{data:data.clone(),mime_type:media_type.clone(),display_name:None}),_=>None }).collect::<Vec<_>>();
            if !native_vision && !attachments.is_empty() { return Err(protocol_error("the selected Copilot model does not accept steering images")); }
            projection.persist_completed_answer(&turn).await?;
            turn.transcript.persist_steering(&message).await?;
            if turn.cancellation.is_cancelled() { return Err(CoreError::Cancelled("Stopped by user".into())); }
            turn.events.send(AgentEvent::Steering { content:message.content.clone() }).await.map_err(protocol_error)?;
            next = MessageOptions::new(redact_user_text(&message.content,&turn.privacy)).with_attachments(attachments);
        }
    }.await;
    turn.cancellation.cancel();
    if let Err(cause) = &run {
        projection.persist_partial(&turn, cause).await?;
        for message in steering {
            if message.recovery_control.is_none() {
                turn.transcript.persist_steering(&message).await?;
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), session.abort()).await;
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), session.disconnect()).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), client.stop()).await;
    match run {
        Ok(()) => {
            // Some reasoning-capable models emit neither readable summaries
            // nor an opaque marker. Only judge absence after reconciliation.
            if expects_reasoning {
                projection.reasoning_unavailable(&turn.events).await?;
            }
            projection.finish(&turn).await
        }
        Err(error) => Err(error),
    }
}

async fn project_event(
    projection: &mut Projection,
    response: &mut super::copilot_response::Response,
    tx: &mpsc::Sender<AgentEvent>,
    seen: &mut EventLedger,
    event: &SessionEvent,
) -> Result<(), CoreError> {
    if event.agent_id.as_deref().is_some_and(|id| !id.is_empty()) {
        return Ok(());
    }
    if !seen.observe(event) {
        return Ok(());
    }
    let data = &event.data;
    match event.event_type.as_str() {
        "assistant.streaming_delta" => projection.network_progress(tx).await?,
        "assistant.message_delta" => {
            let id = data
                .get("messageId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&event.id);
            response.observe_answer(id)?;
            projection
                .delta(
                    tx,
                    id,
                    StreamBlockChannel::Answer,
                    data.get("deltaContent")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default(),
                )
                .await?
        }
        "assistant.reasoning_delta" => {
            let id = response.observe_reasoning(
                data.get("reasoningId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&event.id),
                None,
            )?;
            projection
                .delta(
                    tx,
                    &id,
                    StreamBlockChannel::Thinking,
                    data.get("deltaContent")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default(),
                )
                .await?
        }
        "assistant.reasoning" => {
            let text = data
                .get("content")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| protocol_error("Copilot reasoning has no text content field"))?;
            let id = response.observe_reasoning(
                data.get("reasoningId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&event.id),
                Some(text),
            )?;
            projection.complete_reasoning(tx, &id, text).await?;
        }
        "assistant.message" => {
            if data
                .get("reasoningOpaque")
                .is_some_and(|value| !value.is_null())
                && data
                    .get("reasoningText")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(str::is_empty)
            {
                projection.reasoning_unavailable(tx).await?;
            }
            let text = data
                .get("content")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| protocol_error("Copilot message has no text content field"))?;
            let id = data
                .get("messageId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&event.id);
            let blocks = response.accept(data)?;
            let reasoning_id = response.message_reasoning(
                id,
                data.get("reasoningText")
                    .and_then(serde_json::Value::as_str),
            )?;
            if let (Some(reasoning_id), Some(text)) = (
                reasoning_id,
                data.get("reasoningText")
                    .and_then(serde_json::Value::as_str)
                    .filter(|text| !text.is_empty()),
            ) {
                projection
                    .complete_reasoning(tx, &reasoning_id, text)
                    .await?;
            }
            projection.complete_block(tx, id, text).await?;
            projection.clear_answer();
            if let Some(blocks) = blocks {
                projection.select_answer_blocks(blocks)?;
            }
        }
        "assistant.turn_start" => {
            response.ensure_complete()?;
            for id in response.take_reasoning() {
                projection.mark_persisted(&id);
            }
            response.start(data.get("turnId").and_then(serde_json::Value::as_str));
            projection.clear_answer();
        }
        "assistant.turn_retry" => {
            let turn_id = data
                .get("turnId")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| protocol_error("Copilot retry has no turnId"))?;
            let abandoned = response.retry(turn_id)?;
            projection.discard_answer_blocks(tx, abandoned).await?;
            discard_reasoning(projection, response, tx).await?;
        }
        "tool.execution_start" => {
            response.tool_started();
            projection.clear_answer();
            // Several tool rounds can share one assistant.turn_start. Their
            // successful thinking must not be cleared by a later retry.
            for id in response.take_reasoning() {
                projection.mark_persisted(&id);
            }
        }
        "session.usage_info" => {
            if let Some(used) = data["currentTokens"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
            {
                let capacity = data["tokenLimit"]
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok());
                let mut remaining = used;
                let mut segments = Vec::new();
                for (field, kind) in [
                    ("systemTokens", "systemCore"),
                    ("toolDefinitionsTokens", "tools"),
                    ("conversationTokens", "conversation"),
                ] {
                    if let Some(tokens) = data[field].as_u64().and_then(|v| u32::try_from(v).ok()) {
                        let tokens = tokens.min(remaining);
                        remaining -= tokens;
                        if tokens > 0 {
                            segments.push(nexa_core::agent::context::ContextUsageSegment {
                                kind: kind.into(),
                                tokens,
                            });
                        }
                    }
                }
                if remaining > 0 {
                    segments.push(nexa_core::agent::context::ContextUsageSegment {
                        kind: "overhead".into(),
                        tokens: remaining,
                    });
                }
                projection
                    .context_snapshot(tx, used, capacity, segments)
                    .await?;
            }
        }
        "assistant.usage" => {
            projection.usage_reported |=
                data["inputTokens"].as_u64().is_some() || data["outputTokens"].as_u64().is_some();
            if data
                .get("initiator")
                .and_then(serde_json::Value::as_str)
                .is_none_or(str::is_empty)
            {
                if let Some(model) = data["model"].as_str() {
                    projection.observe_runtime_model(model);
                }
            }
            let tokens = |key: &str| {
                data.get(key)
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0)
                    .min(u32::MAX as u64) as u32
            };
            projection.usage.prompt_tokens = projection
                .usage
                .prompt_tokens
                .saturating_add(tokens("inputTokens"));
            if projection.context_breakdown.is_none()
                && data
                    .get("initiator")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(str::is_empty)
            {
                projection.last_prompt_tokens = tokens("inputTokens");
            }
            projection.usage.completion_tokens = projection
                .usage
                .completion_tokens
                .saturating_add(tokens("outputTokens"));
            projection.usage.total_tokens = projection
                .usage
                .prompt_tokens
                .saturating_add(projection.usage.completion_tokens);
            for (field, target) in [
                ("cacheReadTokens", &mut projection.usage.cache_read_tokens),
                (
                    "cacheWriteTokens",
                    &mut projection.usage.cache_creation_tokens,
                ),
            ] {
                if data[field].as_u64().is_some() {
                    *target = Some(target.unwrap_or(0).saturating_add(tokens(field)));
                }
            }
            projection
                .publish_usage_for_model(tx, data["model"].as_str())
                .await?;
            // Copilot normalizes Anthropic's refusal stop reason to content_filter.
            // Treat the explicit protocol signal as a failed turn, never infer it
            // from assistant prose (which may be quoting an error for the user).
            let main_response = data
                .get("initiator")
                .and_then(serde_json::Value::as_str)
                .is_none_or(str::is_empty);
            if main_response
                && (data
                    .get("contentFilterTriggered")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                    || data.get("finishReason").and_then(serde_json::Value::as_str)
                        == Some("content_filter"))
            {
                projection
                    .discard_answer_blocks(tx, response.abandon_attempt())
                    .await?;
                discard_reasoning(projection, response, tx).await?;
                return Err(protocol_error("Copilot upstream content filtering blocked or truncated this response (content_filter). Nexa did not receive a complete answer. The request was stopped without automatically retrying or switching models."));
            }
        }
        "session.warning" => {
            let message = data["message"]
                .as_str()
                .unwrap_or("Copilot reported a runtime warning")
                .chars()
                .take(2000)
                .collect::<String>();
            if data["warningType"].as_str() == Some("compaction_static_context_blocked") {
                return Err(protocol_error(format!("Copilot: {message}")));
            }
            tx.send(AgentEvent::ControllerStatus {
                code: "external_agent_warning".into(),
                content: message,
                tone: None,
            })
            .await
            .map_err(protocol_error)?;
        }
        "session.error" if event.is_transient_error() => {
            // The SDK keeps model_call errors inside its own retry lifecycle.
            // Aborting here would discard the native retry's eventual answer.
            projection.network_progress(tx).await?;
        }
        "session.error" => {
            let details = [
                "errorType",
                "errorCode",
                "statusCode",
                "providerCallId",
                "serviceRequestId",
            ]
            .iter()
            .filter_map(|key| {
                data.get(key).filter(|value| !value.is_null()).map(|value| {
                    format!(
                        "{key}={}",
                        value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| value.to_string())
                    )
                })
            })
            .collect::<Vec<_>>()
            .join(", ");
            let message = data["message"]
                .as_str()
                .unwrap_or("Copilot session failed")
                .chars()
                .take(2000)
                .collect::<String>();
            return Err(protocol_error(format!("Copilot: {message} [{details}]")));
        }
        "session.compaction_start" => {
            projection.compaction(tx, "copilot", "in_progress").await?;
        }
        "session.compaction_complete" => {
            projection
                .compaction(
                    tx,
                    "copilot",
                    if data["success"] == false {
                        "failed"
                    } else {
                        "completed"
                    },
                )
                .await?;
        }
        _ => {}
    }
    Ok(())
}

async fn discard_reasoning(
    projection: &mut Projection,
    response: &mut super::copilot_response::Response,
    tx: &mpsc::Sender<AgentEvent>,
) -> Result<(), CoreError> {
    for id in response.take_reasoning() {
        projection.complete_reasoning(tx, &id, "").await?;
        projection.mark_persisted(&id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn copilot_static_context_warning_reports_the_cause_before_empty_query_error() {
        let (tx, _rx) = mpsc::channel(16);
        let mut projection = Projection::default();
        let mut response = super::super::copilot_response::Response::default();
        let error = project_test_event(
            &mut projection,
            &mut response,
            &tx,
            "session.warning",
            serde_json::json!({
                "warningType":"compaction_static_context_blocked",
                "message":"Static system messages and tool definitions exceed the model's usable context budget. Reduce static context or switch to a larger-context model."
            }),
        )
        .await
        .expect_err("the native static-context failure must not become a generic empty response");
        assert!(error.to_string().contains("tool definitions"));
        assert!(error.to_string().contains("context budget"));
    }

    #[tokio::test]
    async fn copilot_transient_model_call_error_allows_the_native_retry_answer() {
        let (tx, _rx) = mpsc::channel(16);
        let mut projection = Projection::default();
        let mut response = super::super::copilot_response::Response::default();
        project_test_event(
            &mut projection,
            &mut response,
            &tx,
            "session.error",
            serde_json::json!({"errorType":"model_call","message":"upstream temporarily unavailable"}),
        )
        .await
        .expect("the SDK explicitly defines model_call as non-terminal");
        project_test_event(
            &mut projection,
            &mut response,
            &tx,
            "assistant.message",
            serde_json::json!({"messageId":"retried","content":"Recovered answer"}),
        )
        .await
        .unwrap();
        assert_eq!(projection.answer, "Recovered answer");
    }

    #[test]
    fn sdk_connection_loss_is_recoverable_without_retrying_agent_or_configuration_errors() {
        use github_copilot_sdk::{ErrorKind, ProtocolErrorKind, SessionErrorKind};
        for kind in [
            ErrorKind::Io,
            ErrorKind::Protocol(ProtocolErrorKind::RequestCancelled),
            ErrorKind::Session(SessionErrorKind::EventLoopClosed),
            ErrorKind::Session(SessionErrorKind::Timeout(Duration::from_secs(1))),
        ] {
            assert!(matches!(
                recover_interrupted_runtime(sdk_error(kind.into())),
                CoreError::RecoveryRequired {
                    action_reconciliation: true,
                    ..
                }
            ));
        }
        for kind in [
            ErrorKind::Json,
            ErrorKind::InvalidConfig,
            ErrorKind::Session(SessionErrorKind::AgentError),
            ErrorKind::Rpc { code: 401 },
        ] {
            assert!(matches!(sdk_error(kind.into()), CoreError::Agent(_)));
        }
    }

    #[tokio::test]
    async fn reasoning_offsets_survive_empty_settlement_between_live_events() {
        let (request, mut rx, _, _) =
            super::super::tests::fixture(AgentRuntimeKind::Copilot, "grok-4.7");
        let turn = request.prepare(false).unwrap();
        let mut projection = Projection::for_turn(&turn);
        let mut response = super::super::copilot_response::Response::default();
        for delta in ["first ", "second"] {
            project_test_event(
                &mut projection,
                &mut response,
                &turn.events,
                "assistant.reasoning_delta",
                serde_json::json!({"reasoningId":"reasoning-1","deltaContent":delta}),
            )
            .await
            .unwrap();
            // This is the production event loop, not just its dispatch helper.
            projection
                .persist_settled(&turn, response.take_settled())
                .await
                .unwrap();
        }
        let mut offsets = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let AgentEvent::StreamBlockDelta {
                channel: StreamBlockChannel::Thinking,
                offset,
                delta,
                ..
            } = event
            {
                offsets.push((offset, delta));
            }
        }
        assert_eq!(offsets, vec![(0, "first ".into()), (6, "second".into())]);
    }

    #[tokio::test]
    async fn full_reasoning_event_is_projected_without_requiring_deltas() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut projection = Projection::default();
        let mut response = super::super::copilot_response::Response::default();
        project_test_event(
            &mut projection,
            &mut response,
            &tx,
            "assistant.reasoning",
            serde_json::json!({"reasoningId":"reasoning-1","content":"Visible reasoning snapshot"}),
        )
        .await
        .unwrap();
        let mut visible = false;
        while let Ok(event) = rx.try_recv() {
            visible |= matches!(event,
                AgentEvent::StreamBlockSnapshot {channel: StreamBlockChannel::Thinking, text, ..} if text == "Visible reasoning snapshot");
        }
        assert!(
            visible,
            "Copilot's durable reasoning snapshot must reach the UI"
        );
    }

    #[tokio::test]
    async fn message_reasoning_text_is_visible_but_opaque_replay_is_not() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut projection = Projection::default();
        let mut response = super::super::copilot_response::Response::default();
        project_test_event(&mut projection, &mut response, &tx, "assistant.message",
            serde_json::json!({"messageId":"message-1","content":"Final answer",
                "reasoningText":"Readable rationale", "reasoningOpaque":"OPAQUE_SENTINEL", "encryptedContent":"ENCRYPTED_SENTINEL"}),
        ).await.unwrap();
        let mut texts = Vec::new();
        while let Ok(event) = rx.try_recv() {
            match event {
                AgentEvent::StreamBlockSnapshot {
                    channel: StreamBlockChannel::Thinking,
                    text,
                    ..
                } => texts.push(text),
                AgentEvent::StreamBlockDelta {
                    channel: StreamBlockChannel::Thinking,
                    delta,
                    ..
                } => texts.push(delta),
                _ => {}
            }
        }
        assert_eq!(texts, vec!["Readable rationale"]);
        assert_eq!(projection.answer, "Final answer");
    }

    #[tokio::test]
    async fn reasoning_snapshots_repair_one_block_and_answers_stream_before_idle() {
        let (request, mut rx, _, _) =
            super::super::tests::fixture(AgentRuntimeKind::Copilot, "grok-4.7");
        let turn = request.prepare(false).unwrap();
        let mut projection = Projection::for_turn(&turn);
        let mut response = super::super::copilot_response::Response::default();
        let mut thinking = std::collections::HashMap::<String, String>::new();
        let mut answer = String::new();
        for (kind, data) in [
            (
                "assistant.reasoning_delta",
                serde_json::json!({"reasoningId":"r","deltaContent":"错"}),
            ),
            (
                "session.usage_info",
                serde_json::json!({"currentTokens":10}),
            ),
            (
                "assistant.reasoning_delta",
                serde_json::json!({"reasoningId":"r","deltaContent":"误"}),
            ),
            (
                "assistant.reasoning",
                serde_json::json!({"reasoningId":"r","content":"修正思考🙂"}),
            ),
            (
                "assistant.message_delta",
                serde_json::json!({"messageId":"a","deltaContent":"实时"}),
            ),
            (
                "assistant.message_delta",
                serde_json::json!({"messageId":"a","deltaContent":"回答"}),
            ),
            (
                "assistant.message",
                serde_json::json!({"messageId":"a","content":"实时回答","reasoningText":"修正思考🙂"}),
            ),
        ] {
            project_test_event(&mut projection, &mut response, &turn.events, kind, data)
                .await
                .unwrap();
            projection
                .persist_settled(&turn, response.take_settled())
                .await
                .unwrap();
            let mut answer_delta = false;
            while let Ok(event) = rx.try_recv() {
                match event {
                    AgentEvent::StreamBlockDelta {
                        block_id,
                        channel: StreamBlockChannel::Thinking,
                        offset,
                        delta,
                    } => {
                        let text = thinking.entry(block_id).or_default();
                        assert_eq!(offset, text.len());
                        text.push_str(&delta);
                    }
                    AgentEvent::StreamBlockSnapshot {
                        block_id,
                        channel: StreamBlockChannel::Thinking,
                        text,
                    } => {
                        thinking.insert(block_id, text);
                    }
                    AgentEvent::StreamBlockDelta {
                        channel: StreamBlockChannel::Answer,
                        offset,
                        delta,
                        ..
                    } => {
                        assert_eq!(offset, answer.len());
                        answer.push_str(&delta);
                        answer_delta = true;
                    }
                    _ => {}
                }
            }
            if kind == "assistant.message_delta" {
                assert!(
                    answer_delta,
                    "each answer delta must reach the UI before full-message/idle"
                );
            }
        }
        assert_eq!(thinking.len(), 1);
        assert_eq!(thinking.values().next().unwrap(), "修正思考🙂");
        assert_eq!(answer, "实时回答");
        assert_eq!(projection.answer, answer);
    }

    #[tokio::test]
    async fn reasoning_retry_clears_abandoned_blocks_and_turns_retire_their_state() {
        let (tx, mut rx) = mpsc::channel(32);
        let mut projection = Projection::default();
        let mut response = super::super::copilot_response::Response::default();
        for (kind, data) in [
            ("assistant.turn_start", serde_json::json!({"turnId":"t"})),
            (
                "assistant.reasoning_delta",
                serde_json::json!({"reasoningId":"r","deltaContent":"abandoned"}),
            ),
            ("assistant.turn_retry", serde_json::json!({"turnId":"t"})),
            (
                "assistant.reasoning",
                serde_json::json!({"reasoningId":"r","content":"late replay"}),
            ),
            (
                "assistant.reasoning_delta",
                serde_json::json!({"reasoningId":"new","deltaContent":"fresh"}),
            ),
        ] {
            project_test_event(&mut projection, &mut response, &tx, kind, data)
                .await
                .unwrap();
        }
        let mut visible = std::collections::HashMap::<String, String>::new();
        while let Ok(event) = rx.try_recv() {
            match event {
                AgentEvent::StreamBlockDelta {
                    block_id, delta, ..
                } => {
                    visible.entry(block_id).or_default().push_str(&delta);
                }
                AgentEvent::StreamBlockSnapshot { block_id, text, .. } => {
                    visible.insert(block_id, text);
                }
                _ => {}
            }
        }
        assert_eq!(visible["copilot:reasoning:r"], "");
        assert_eq!(visible["copilot:reasoning:new"], "fresh");
        project_test_event(
            &mut projection,
            &mut response,
            &tx,
            "assistant.turn_retry",
            serde_json::json!({"turnId":"t"}),
        )
        .await
        .unwrap();
        project_test_event(
            &mut projection,
            &mut response,
            &tx,
            "assistant.message",
            serde_json::json!({"messageId":"answer", "content":"retry succeeded"}),
        )
        .await
        .unwrap();
        projection.reasoning_unavailable(&tx).await.unwrap();
        let mut unavailable = 0;
        while let Ok(event) = rx.try_recv() {
            if matches!(event, AgentEvent::ControllerStatus {code, ..} if code == "external_agent_reasoning_unavailable")
            {
                unavailable += 1;
            }
        }
        assert_eq!(
            unavailable, 1,
            "discarded thinking cannot suppress the final visibility notice"
        );
        // A long tool-driven session must release reasoning bookkeeping at
        // genuine inference boundaries, even when there was no answer draft.
        for index in 0..2100 {
            project_test_event(
                &mut projection,
                &mut response,
                &tx,
                "assistant.turn_start",
                serde_json::json!({"turnId":format!("turn-{index}")}),
            )
            .await
            .unwrap();
            project_test_event(
                &mut projection,
                &mut response,
                &tx,
                "assistant.reasoning",
                serde_json::json!({"reasoningId":format!("r-{index}"),"content":"thought"}),
            )
            .await
            .unwrap();
            while rx.try_recv().is_ok() {}
        }
    }

    #[tokio::test]
    async fn opaque_only_messages_and_subagent_reasoning_never_enter_parent_thinking() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut projection = Projection::default();
        let mut response = super::super::copilot_response::Response::default();
        let event: SessionEvent = serde_json::from_value(serde_json::json!({
            "id":"child", "timestamp":"2026-10-09T00:00:00Z", "type":"assistant.reasoning",
            "agentId":"child-agent", "data":{"reasoningId":"r", "content":"child thought"}
        }))
        .unwrap();
        project_event(
            &mut projection,
            &mut response,
            &tx,
            &mut EventLedger::default(),
            &event,
        )
        .await
        .unwrap();
        project_test_event(&mut projection, &mut response, &tx, "assistant.message",
            serde_json::json!({"messageId":"a", "content":"answer", "reasoningOpaque":"secret", "encryptedContent":"secret"})).await.unwrap();
        project_test_event(
            &mut projection,
            &mut response,
            &tx,
            "assistant.message",
            serde_json::json!({"messageId":"b", "content":"continued", "reasoningOpaque":"secret"}),
        )
        .await
        .unwrap();
        let mut notices = 0;
        while let Ok(event) = rx.try_recv() {
            if let AgentEvent::ControllerStatus { code, content, .. } = &event {
                if code == "external_agent_reasoning_unavailable" {
                    notices += 1;
                }
                assert!(!content.contains("secret"));
            }
            assert!(!matches!(
                event,
                AgentEvent::StreamBlockDelta {
                    channel: StreamBlockChannel::Thinking,
                    ..
                } | AgentEvent::StreamBlockSnapshot {
                    channel: StreamBlockChannel::Thinking,
                    ..
                }
            ));
        }
        assert_eq!(
            notices, 1,
            "opaque-only responses need one honest visibility notice"
        );
        for snapshot in [false, true] {
            let mut readable = Projection::default();
            if snapshot {
                readable
                    .complete_reasoning(&tx, "r", "visible summary")
                    .await
                    .unwrap();
            } else {
                readable
                    .delta(&tx, "r", StreamBlockChannel::Thinking, "visible summary")
                    .await
                    .unwrap();
            }
            readable.reasoning_unavailable(&tx).await.unwrap();
            while let Ok(event) = rx.try_recv() {
                assert!(
                    !matches!(event, AgentEvent::ControllerStatus {code, ..} if code == "external_agent_reasoning_unavailable")
                );
            }
        }
    }

    #[tokio::test]
    async fn reasoning_snapshot_after_message_reuses_fallback_when_deltas_were_absent() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut projection = Projection::default();
        let mut response = super::super::copilot_response::Response::default();
        for (kind, data) in [
            (
                "assistant.message",
                serde_json::json!({"messageId":"a", "content":"answer", "reasoningText":"thought"}),
            ),
            (
                "assistant.reasoning",
                serde_json::json!({"reasoningId":"r", "content":"thought"}),
            ),
        ] {
            project_test_event(&mut projection, &mut response, &tx, kind, data)
                .await
                .unwrap();
        }
        let mut ids = HashSet::new();
        while let Ok(event) = rx.try_recv() {
            if let AgentEvent::StreamBlockSnapshot {
                block_id,
                channel: StreamBlockChannel::Thinking,
                text,
            } = event
            {
                assert_eq!(text, "thought");
                ids.insert(block_id);
            }
        }
        assert_eq!(
            ids.len(),
            1,
            "late full reasoning must not duplicate the message fallback"
        );
    }

    #[tokio::test]
    async fn reasoning_cleanup_preserves_settled_tool_rounds_and_clears_rejected_attempts() {
        let mut observed = Vec::new();
        for (kind, signal) in [
            ("assistant.turn_retry", serde_json::json!({"turnId":"turn"})),
            (
                "assistant.usage",
                serde_json::json!({"contentFilterTriggered":true}),
            ),
            (
                "assistant.usage",
                serde_json::json!({"finishReason":"content_filter"}),
            ),
        ] {
            let (tx, mut rx) = mpsc::channel(32);
            let mut projection = Projection::default();
            let mut response = super::super::copilot_response::Response::default();
            for (kind, data) in [
                ("assistant.turn_start", serde_json::json!({"turnId":"turn"})),
                (
                    "assistant.reasoning",
                    serde_json::json!({"reasoningId":"settled", "content":"valid earlier thought"}),
                ),
                (
                    "assistant.message",
                    serde_json::json!({"messageId":"tool", "content":"Checking", "toolRequests":[{}]}),
                ),
                ("tool.execution_start", serde_json::json!({})),
                (
                    "assistant.reasoning_delta",
                    serde_json::json!({"reasoningId":"active", "deltaContent":"rejected thought"}),
                ),
            ] {
                project_test_event(&mut projection, &mut response, &tx, kind, data)
                    .await
                    .unwrap();
            }
            let result =
                project_test_event(&mut projection, &mut response, &tx, kind, signal).await;
            assert_eq!(result.is_err(), kind == "assistant.usage");
            let mut visible = std::collections::HashMap::<String, String>::new();
            while let Ok(event) = rx.try_recv() {
                match event {
                    AgentEvent::StreamBlockDelta {
                        block_id,
                        channel: StreamBlockChannel::Thinking,
                        delta,
                        ..
                    } => {
                        visible.entry(block_id).or_default().push_str(&delta);
                    }
                    AgentEvent::StreamBlockSnapshot {
                        block_id,
                        channel: StreamBlockChannel::Thinking,
                        text,
                    } => {
                        visible.insert(block_id, text);
                    }
                    _ => {}
                }
            }
            observed.push((
                visible["copilot:reasoning:settled"].clone(),
                visible["copilot:reasoning:active"].clone(),
            ));
        }
        assert_eq!(
            observed,
            vec![("valid earlier thought".to_string(), String::new()); 3]
        );
    }

    #[tokio::test]
    #[ignore = "uses an explicitly selected official Copilot model for one synthetic streaming inference"]
    async fn native_copilot_streams_readable_reasoning_and_answer_before_done() {
        native_copilot_stream_probe(true).await;
    }

    #[tokio::test]
    #[ignore = "uses an explicitly selected Copilot model without readable summaries for one synthetic inference"]
    async fn native_copilot_reports_unavailable_readable_reasoning() {
        native_copilot_stream_probe(false).await;
    }

    async fn native_copilot_stream_probe(expect_readable: bool) {
        let model = std::env::var("NEXA_COPILOT_STREAM_PROBE_MODEL")
            .expect("set the intended reasoning model explicitly");
        let (mut request, mut rx, _, _) =
            super::super::tests::fixture(AgentRuntimeKind::Copilot, &model);
        if let Ok(effort) = std::env::var("NEXA_COPILOT_STREAM_PROBE_EFFORT") {
            request.config.reasoning_effort = Some(
                serde_json::from_value(serde_json::Value::String(effort))
                    .expect("set a supported reasoning effort explicitly"),
            );
        }
        request.dependencies.tools = nexa_core::tools::ToolRegistry::new();
        request.config.system_prompt =
            "Solve the synthetic arithmetic problem accurately and concisely. Do not use tools."
                .into();
        request.user_parts = vec![ContentPart::Text {
            text: if expect_readable {
                // A trivial arithmetic prompt can legitimately skip adaptive
                // thinking. Match the selected model/effort with a task that
                // exercises reasoning rather than infer support from absence.
                "Count the triples of nonnegative integers x,y,z satisfying x+2y+3z=47, x<=17 and y+z<=20. Return the count and a concise explanation."
            } else {
                "What is 137 times 263? Give the result and a brief explanation in three sentences."
            }.into(),
        }];
        let cancellation = request.cancellation.clone();
        let started = std::time::Instant::now();
        let operation = run(request);
        tokio::pin!(operation);
        let mut completed = false;
        let mut done = false;
        let mut thinking_deltas = 0;
        let mut answer_deltas = 0;
        let mut snapshots = 0;
        let mut opaque_notices = 0;
        let mut visible = std::collections::HashMap::<String, String>::new();
        let timeout = tokio::time::sleep(Duration::from_secs(180));
        tokio::pin!(timeout);
        while !completed || !done {
            tokio::select! {
                _ = &mut timeout => { cancellation.cancel(); panic!("Copilot streaming probe timed out"); }
                result = &mut operation, if !completed => { result.unwrap(); completed = true; }
                event = rx.recv() => match event.expect("runtime event channel closed before Done") {
                    AgentEvent::StreamBlockDelta {block_id, channel, offset, delta} => {
                        assert!(!done, "streaming must precede Done");
                        let text = visible.entry(block_id).or_default();
                        assert_eq!(offset, text.len(), "live offsets must remain contiguous");
                        text.push_str(&delta);
                        if channel == StreamBlockChannel::Thinking { thinking_deltas += 1; }
                        else { answer_deltas += 1; }
                    }
                    AgentEvent::StreamBlockSnapshot {block_id, channel, text} => {
                        if channel == StreamBlockChannel::Thinking { snapshots += 1; }
                        visible.insert(block_id, text);
                    }
                    AgentEvent::Done {..} => done = true,
                    AgentEvent::ControllerStatus { code, .. } if code == "external_agent_reasoning_unavailable" => opaque_notices += 1,
                    _ => {}
                }
            }
        }
        eprintln!("Copilot live projection: model={model} elapsed_ms={} thinking_deltas={thinking_deltas} answer_deltas={answer_deltas} thinking_snapshots={snapshots} opaque_notices={opaque_notices}", started.elapsed().as_millis());
        // Live models may return only a readable full snapshot on a given
        // request. Deterministic regressions above enforce delta continuity;
        // this probe verifies actual readable thinking and streamed answers.
        assert!(answer_deltas > 1, "the answer must stream before Done");
        if expect_readable {
            assert!(thinking_deltas + snapshots > 0);
        } else {
            assert_eq!(thinking_deltas + snapshots, 0);
            assert_eq!(
                opaque_notices, 1,
                "missing upstream summaries must be explained, never shown as readable thoughts"
            );
        }
    }

    #[tokio::test]
    async fn native_context_snapshot_survives_compaction_and_done_without_becoming_billable() {
        let (request, mut rx, _, _) =
            super::super::tests::fixture(AgentRuntimeKind::Copilot, "test");
        let turn = request.prepare(false).unwrap();
        let mut projection = Projection::default();
        let mut response = super::super::copilot_response::Response::default();
        for current in [48_000, 12_000] {
            project_test_event(
                &mut projection,
                &mut response,
                &turn.events,
                "session.usage_info",
                serde_json::json!({
                    "currentTokens": current, "tokenLimit": 200_000, "messagesLength": 4,
                    "systemTokens": 2_000, "toolDefinitionsTokens": 3_000,
                    "conversationTokens": current - 5_000
                }),
            )
            .await
            .unwrap();
            let event = rx
                .try_recv()
                .expect("native context usage must reach the UI live");
            let wire = nexa_core::agent_run::AgentRunEvent::from_agent_event(&event);
            assert_eq!(wire.payload["lastPromptTokens"], current);
            assert_eq!(wire.payload["contextBreakdown"]["contextWindow"], 200_000);
            assert_eq!(
                projection.usage.total_tokens, 0,
                "context is not billed usage"
            );
        }
        project_test_event(
            &mut projection,
            &mut response,
            &turn.events,
            "assistant.message",
            serde_json::json!({"messageId":"answer","content":"done"}),
        )
        .await
        .unwrap();
        projection.finish(&turn).await.unwrap();
        let mut done = None;
        while let Ok(event) = rx.try_recv() {
            if matches!(event, AgentEvent::Done { .. }) {
                done = Some(nexa_core::agent_run::AgentRunEvent::from_agent_event(
                    &event,
                ));
            }
        }
        let done = done.unwrap();
        assert_eq!(done.payload["lastPromptTokens"], 12_000);
        assert_eq!(done.payload["contextBreakdown"]["contextWindow"], 200_000);
    }

    #[tokio::test]
    async fn explicit_content_filter_clears_the_failed_attempt_without_classifying_prose() {
        for signal in [
            serde_json::json!({"contentFilterTriggered":true}),
            serde_json::json!({"finishReason":"content_filter"}),
        ] {
            let (tx, mut rx) = mpsc::channel(16);
            let mut projection = Projection::default();
            let mut response = super::super::copilot_response::Response::default();
            let notice = "The model returned no content because the response was blocked by content filtering.";
            project_test_event(
                &mut projection,
                &mut response,
                &tx,
                "assistant.message",
                serde_json::json!({"messageId":"filtered","content":notice}),
            )
            .await
            .unwrap();
            // Text alone is a valid quote; only the structured signal rejects it.
            assert_eq!(projection.answer, notice);
            project_test_event(
                &mut projection,
                &mut response,
                &tx,
                "assistant.usage",
                serde_json::json!({"contentFilterTriggered":true,"initiator":"sub-agent"}),
            )
            .await
            .unwrap();
            assert_eq!(
                projection.answer, notice,
                "background usage cannot fail the parent response"
            );
            let error = project_test_event(
                &mut projection,
                &mut response,
                &tx,
                "assistant.usage",
                signal,
            )
            .await
            .unwrap_err();
            assert!(error
                .to_string()
                .contains("Copilot upstream content filtering"));
            assert!(projection.answer.is_empty());
            let mut cleared = false;
            while let Ok(event) = rx.try_recv() {
                if let AgentEvent::StreamBlockSnapshot { block_id, text, .. } = event {
                    cleared |= block_id == "filtered" && text.is_empty();
                }
            }
            assert!(cleared, "the UI must discard the filtered attempt too");
        }
    }
    #[test]
    fn empty_tool_mode_preserves_normal_system_keychain_login() {
        let options = with_login_credential_backend(
            super::super::copilot_options::client_options("copilot".into())
                .with_mode(ClientMode::Empty),
            None,
        );
        assert_eq!(options.mode, ClientMode::Empty);
        assert!(options
            .env_remove
            .contains(&std::ffi::OsString::from("COPILOT_DISABLE_KEYTAR")));
        assert!(options.github_token.is_none());
    }
    #[test]
    fn explicit_login_keychain_setting_is_preserved_without_copying_credentials() {
        for value in ["0", "1", ""] {
            let options = with_login_credential_backend(
                super::super::copilot_options::client_options("copilot".into())
                    .with_mode(ClientMode::Empty),
                Some(value.into()),
            );
            assert_eq!(options.mode, ClientMode::Empty);
            assert!(options
                .env
                .iter()
                .any(|(key, current)| key == "COPILOT_DISABLE_KEYTAR" && current == value));
            assert!(!options
                .env_remove
                .contains(&std::ffi::OsString::from("COPILOT_DISABLE_KEYTAR")));
            assert!(options.github_token.is_none());
            assert!(options
                .env
                .iter()
                .any(|(key, current)| key == "GITHUB_COPILOT_INTEGRATION_ID"
                    && current == "vscode-chat"));
        }
    }
    async fn project_test_event(
        projection: &mut Projection,
        response: &mut super::super::copilot_response::Response,
        tx: &mpsc::Sender<AgentEvent>,
        kind: &str,
        data: serde_json::Value,
    ) -> Result<(), CoreError> {
        let event: SessionEvent = serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(), "timestamp": "2026-09-05T00:00:00Z", "type": kind, "data": data
        })).unwrap();
        project_event(
            projection,
            response,
            tx,
            &mut EventLedger::default(),
            &event,
        )
        .await
    }

    #[tokio::test]
    async fn split_response_is_ordered_corrected_and_checkpointed_without_duplicate_chunks() {
        let (request, mut rx, _, _) =
            super::super::tests::fixture(AgentRuntimeKind::Copilot, "test");
        let db = request.db.clone();
        let conversation = request.conversation_id.clone();
        let turn = request.prepare(false).unwrap();
        let mut projection = Projection::default();
        let mut response = super::super::copilot_response::Response::default();
        for (index, content) in [
            (3, ""),
            (1, "first draft"),
            (0, ""),
            (2, "第二段"),
            (1, "第一段"),
        ] {
            project_test_event(&mut projection, &mut response, &turn.events, "assistant.message", serde_json::json!({
                "apiCallId": "api", "messageId": format!("m{index}"), "chunkIndex": index, "chunkCount": 4,
                "content": content, "reasoningText": "private reasoning"
            })).await.unwrap();
        }
        response.ensure_complete().unwrap();
        assert_eq!(projection.answer, "第一段\n\n第二段");
        projection
            .persist_completed_answer(&turn)
            .await
            .unwrap()
            .unwrap();
        turn.transcript
            .persist_steering(&AgentSteeringMessage::text("follow up"))
            .await
            .unwrap();
        projection
            .persist_partial(
                &turn,
                &nexa_core::error::CoreError::StreamIncomplete("fixture interruption".into()),
            )
            .await
            .unwrap();
        let history = db.get_messages(&conversation).unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[1].content, "第一段\n\n第二段");
        assert_eq!(history[2].content, "follow up");
        while let Ok(event) = rx.try_recv() {
            assert!(!matches!(event, AgentEvent::Done { .. }));
        }
    }

    #[tokio::test]
    async fn incomplete_response_never_becomes_final_or_crosses_api_boundaries() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut projection = Projection::default();
        let mut response = super::super::copilot_response::Response::default();
        assert!(project_test_event(
            &mut projection,
            &mut response,
            &tx,
            "assistant.message",
            serde_json::json!({
                "apiCallId": "malformed", "messageId": "missing-content"
            })
        )
        .await
        .is_err());
        for index in [0, 2] {
            project_test_event(&mut projection, &mut response, &tx, "assistant.message", serde_json::json!({
                "apiCallId": "api", "messageId": format!("m{index}"), "chunkIndex": index, "chunkCount": 3, "content": "part"
            })).await.unwrap();
        }
        assert!(projection.answer.is_empty());
        assert!(response.ensure_complete().is_err());
        assert!(project_test_event(
            &mut projection,
            &mut response,
            &tx,
            "assistant.message",
            serde_json::json!({
                "apiCallId": "different-api", "messageId": "other", "content": "other answer"
            })
        )
        .await
        .is_err());
        while let Ok(event) = rx.try_recv() {
            assert!(!matches!(event, AgentEvent::Done { .. }));
        }
    }

    #[tokio::test]
    async fn tool_commentary_and_retried_chunks_do_not_leak_into_final_answer() {
        let (tx, _rx) = mpsc::channel(32);
        let mut projection = Projection::default();
        let mut response = super::super::copilot_response::Response::default();
        for (kind, data) in [
            ("assistant.turn_start", serde_json::json!({"turnId":"turn"})),
            (
                "assistant.message",
                serde_json::json!({"apiCallId":"tool-api", "messageId":"comment", "content":"Checking", "toolRequests":[{}]}),
            ),
            ("tool.execution_start", serde_json::json!({})),
            (
                "assistant.message",
                serde_json::json!({"messageId":"abandoned", "chunkIndex":0,"chunkCount":2,"content":"retry draft"}),
            ),
            ("assistant.turn_retry", serde_json::json!({"turnId":"turn"})),
            (
                "assistant.message",
                serde_json::json!({"messageId":"a", "chunkIndex":0,"chunkCount":2,"content":"final"}),
            ),
            (
                "assistant.message",
                serde_json::json!({"messageId":"b", "chunkIndex":1,"chunkCount":2,"content":"answer"}),
            ),
        ] {
            project_test_event(&mut projection, &mut response, &tx, kind, data)
                .await
                .unwrap();
        }
        response.ensure_complete().unwrap();
        assert_eq!(projection.answer, "final\n\nanswer");
    }

    #[tokio::test]
    async fn failed_retry_discards_completed_and_delta_only_blocks_but_preserves_prior_response() {
        let (request, mut rx, _, _) =
            super::super::tests::fixture(AgentRuntimeKind::Copilot, "test");
        let db = request.db.clone();
        let conversation = request.conversation_id.clone();
        let turn = request.prepare(false).unwrap();
        let mut projection = Projection::default();
        let mut response = super::super::copilot_response::Response::default();
        for (kind, data) in [
            (
                "assistant.turn_start",
                serde_json::json!({"turnId":"first"}),
            ),
            (
                "assistant.message",
                serde_json::json!({"messageId":"saved","content":"saved answer"}),
            ),
        ] {
            project_test_event(&mut projection, &mut response, &turn.events, kind, data)
                .await
                .unwrap();
        }
        projection
            .persist_completed_answer(&turn)
            .await
            .unwrap()
            .unwrap();
        turn.transcript
            .persist_steering(&AgentSteeringMessage::text("follow up"))
            .await
            .unwrap();
        for (kind, data) in [
            (
                "assistant.turn_start",
                serde_json::json!({"turnId":"second"}),
            ),
            (
                "assistant.message_delta",
                serde_json::json!({"messageId":"delta-only","deltaContent":"abandoned unfinished text"}),
            ),
            (
                "assistant.message",
                serde_json::json!({"apiCallId":"bad","messageId":"reused","chunkIndex":0,"chunkCount":2,"content":"abandoned completed chunk"}),
            ),
            (
                "assistant.turn_retry",
                serde_json::json!({"turnId":"second"}),
            ),
            (
                "assistant.message_delta",
                serde_json::json!({"messageId":"reused","deltaContent":"replacement partial"}),
            ),
        ] {
            project_test_event(&mut projection, &mut response, &turn.events, kind, data)
                .await
                .unwrap();
        }
        assert!(project_test_event(
            &mut projection,
            &mut response,
            &turn.events,
            "session.error",
            serde_json::json!({"message":"connection failed"})
        )
        .await
        .is_err());
        projection
            .persist_partial(
                &turn,
                &nexa_core::error::CoreError::StreamIncomplete("fixture interruption".into()),
            )
            .await
            .unwrap();
        let history = db.get_messages(&conversation).unwrap();
        assert_eq!(
            history[1..]
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            vec!["saved answer", "follow up", "replacement partial"]
        );
        let mut cleared = HashSet::new();
        let mut replacement_offset = None;
        while let Ok(event) = rx.try_recv() {
            nexa_core::agent_run::AgentRunEvent::from_agent_event(&event)
                .with_context(Some("retry-run"), Some("retry-turn"), Some(1))
                .validate_durable_contract()
                .unwrap();
            match event {
                AgentEvent::StreamBlockSnapshot { block_id, text, .. } if text.is_empty() => {
                    cleared.insert(block_id);
                }
                AgentEvent::StreamBlockDelta { offset, delta, .. }
                    if delta == "replacement partial" =>
                {
                    replacement_offset = Some(offset)
                }
                AgentEvent::Done { .. } => panic!("failed retry cannot complete successfully"),
                _ => {}
            }
        }
        assert_eq!(
            cleared,
            HashSet::from(["delta-only".to_string(), "reused".to_string()])
        );
        assert_eq!(replacement_offset, Some(0));
    }

    #[tokio::test]
    #[ignore = "uses the official Copilot subscription to edit one disposable temporary file"]
    async fn native_copilot_edits_with_full_nexa_catalog() {
        let binary = crate::commands::subscription_accounts::resolve_copilot_binary().unwrap();
        let client = Client::start(client_options(binary).unwrap())
            .await
            .unwrap();
        let models = client.list_models().await.unwrap();
        let selected_model = std::env::var("NEXA_COPILOT_LIVE_MODEL")
            .unwrap_or_else(|_| "claude-opus-5.5".to_string());
        let model = models
            .iter()
            .find(|model| model.id == selected_model)
            .expect("explicitly selected live model must be available")
            .id
            .clone();
        client.stop().await.unwrap();
        super::super::tests::run_live_edit(AgentRuntimeKind::Copilot, &model).await;
    }

    #[tokio::test]
    #[ignore = "uses the official Copilot subscription for one read-only tool inference"]
    async fn native_copilot_executes_nexa_tool_and_streams_persisted_answer() {
        let binary = crate::commands::subscription_accounts::resolve_copilot_binary().unwrap();
        let client = Client::start(client_options(binary).unwrap())
            .await
            .unwrap();
        let models = client.list_models().await.unwrap();
        let model = models
            .iter()
            .find(|model| model.id == "gpt-5-mini")
            .or_else(|| models.first())
            .unwrap()
            .id
            .clone();
        client.stop().await.unwrap();
        super::super::tests::run_live(AgentRuntimeKind::Copilot, &model).await;
    }
}
