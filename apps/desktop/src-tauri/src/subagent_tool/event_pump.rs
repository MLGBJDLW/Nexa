use super::*;
use crate::agent_stream_bridge::{event_has_visible_token, event_marks_provider_response_byte};

pub(super) struct SubagentEventPumpConfig {
    pub(super) cancel_token: CancellationToken,
    pub(super) worker_actual_token_limit: Option<u32>,
    pub(super) telemetry_db: Database,
    pub(super) telemetry_identity: Option<(String, String)>,
    pub(super) telemetry_call_label: String,
    pub(super) lifecycle: Option<SubagentEventBridge>,
    pub(super) launch_started: Instant,
    pub(super) non_streaming_completion: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subagent_lifecycle::{RegisterSubagentRequest, SubagentLifecycleRuntime};
    use nexa_core::activity::ActivityRuntime;
    use nexa_core::agent::StreamBlockChannel;
    use nexa_core::conversation::CreateConversationInput;

    #[tokio::test]
    async fn subagent_pump_persists_readable_snapshots_across_privacy_and_drains_the_last_typed_event(
    ) {
        let db = Database::open_memory().unwrap();
        let conversation = db
            .create_conversation(&CreateConversationInput {
                provider: "open_ai".into(),
                model: "test".into(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap();
        db.save_privacy_config(&nexa_core::privacy::PrivacyConfig {
            enabled: true,
            redact_patterns: vec![nexa_core::privacy::RedactRule {
                name: "test".into(),
                pattern: "privateCODE".into(),
                replacement: "[PRIVATE]".into(),
            }],
            ..Default::default()
        })
        .unwrap();
        let lifecycle = SubagentLifecycleRuntime::default();
        let cancel = CancellationToken::new();
        let registration = lifecycle
            .register(RegisterSubagentRequest {
                agent_id: "child".into(),
                parent_call_id: "parent-call".into(),
                task: "Inspect a fixture".into(),
                role_id: None,
                role: None,
                conversation_id: Some(conversation.id.clone()),
                turn_id: None,
                task_run_id: None,
                cancel_token: cancel.clone(),
                activity_runtime: ActivityRuntime::with_database(db.clone()).unwrap(),
            })
            .unwrap();
        registration.events.start().await.unwrap();
        let (tx, rx) = mpsc::channel(64);
        let pump = SubagentEventPump::spawn(
            rx,
            SubagentEventPumpConfig {
                cancel_token: cancel,
                worker_actual_token_limit: None,
                telemetry_db: db.clone(),
                telemetry_identity: None,
                telemetry_call_label: "fixture".into(),
                lifecycle: Some(registration.events),
                launch_started: Instant::now(),
                non_streaming_completion: false,
            },
        );
        tx.send(AgentEvent::Thinking {
            content: "private".into(),
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(130)).await;
        tx.send(AgentEvent::Thinking {
            content: "CODE".into(),
        })
        .await
        .unwrap();
        tx.send(AgentEvent::StreamBlockDelta {
            block_id: "answer".into(),
            channel: StreamBlockChannel::Answer,
            offset: 0,
            delta: "你好".into(),
        })
        .await
        .unwrap();
        tx.send(AgentEvent::StreamBlockDelta {
            block_id: "answer".into(),
            channel: StreamBlockChannel::Answer,
            offset: 6,
            delta: "，完成".into(),
        })
        .await
        .unwrap();
        drop(tx);
        pump.task.await.unwrap();
        let page = db
            .read_subagent_history(&conversation.id, "child", 0)
            .unwrap();
        let serialized = serde_json::to_string(&page).unwrap();
        assert!(!serialized.contains("privateCODE"));
        assert!(page
            .events
            .iter()
            .any(|event| event.payload["detail"]["event"]["text"] == "[PRIVATE]"));
        assert!(page
            .events
            .iter()
            .any(|event| event.payload["detail"]["event"]["text"] == "你好，完成"));
        assert!(!page
            .events
            .iter()
            .any(|event| event.payload["subagentEvent"] == "thinkingDelta"));
    }
}

pub(super) struct SubagentEventPumpHandle {
    pub(super) task: tokio::task::JoinHandle<EventCapture>,
    pub(super) fatal_error_rx: mpsc::UnboundedReceiver<String>,
}

pub(super) struct SubagentEventPump;

impl SubagentEventPump {
    pub(super) fn spawn(
        event_rx: mpsc::Receiver<AgentEvent>,
        config: SubagentEventPumpConfig,
    ) -> SubagentEventPumpHandle {
        let (fatal_error_tx, fatal_error_rx) = mpsc::unbounded_channel::<String>();
        let task = tokio::spawn(Self::run(event_rx, fatal_error_tx, config));
        SubagentEventPumpHandle {
            task,
            fatal_error_rx,
        }
    }

    async fn run(
        event_rx: mpsc::Receiver<AgentEvent>,
        fatal_error_tx: mpsc::UnboundedSender<String>,
        config: SubagentEventPumpConfig,
    ) -> EventCapture {
        let SubagentEventPumpConfig {
            cancel_token: capture_cancel_token,
            worker_actual_token_limit,
            telemetry_db,
            telemetry_identity,
            telemetry_call_label,
            lifecycle: lifecycle_capture,
            launch_started,
            non_streaming_completion,
        } = config;
        let mut event_rx = event_rx;
        let mut capture = EventCapture::default();
        let mut provider_invocation_index = 0_u32;
        let mut active_provider_invocation_id: Option<String> = None;
        let mut first_provider_byte_recorded = false;
        let mut first_visible_token_recorded = false;
        let mut pending_thinking = String::new();
        let mut pending_output = String::new();
        let mut transcript = SubagentTranscript::default();
        let mut canonical_thinking = false;
        let mut canonical_output = false;
        let mut last_delta_flush = Instant::now();
        let mut worker_token_limit_exceeded = false;
        loop {
            // Dormant workers sleep on their channel. A flush timer exists
            // only while a delta is buffered, eliminating ten idle wakeups
            // per second per worker during long provider/tool requests.
            let next = if pending_thinking.is_empty()
                && pending_output.is_empty()
                && !transcript.has_pending()
            {
                Ok(event_rx.recv().await)
            } else {
                let remaining =
                    Duration::from_millis(100).saturating_sub(last_delta_flush.elapsed());
                tokio::time::timeout(remaining, event_rx.recv()).await
            };
            let event = match next {
                Ok(Some(event)) => event,
                Ok(None) => {
                    flush_subagent_deltas(
                        lifecycle_capture.as_ref(),
                        &mut pending_thinking,
                        &mut pending_output,
                        &mut transcript,
                    )
                    .await;
                    break;
                }
                Err(_) => {
                    flush_subagent_deltas(
                        lifecycle_capture.as_ref(),
                        &mut pending_thinking,
                        &mut pending_output,
                        &mut transcript,
                    )
                    .await;
                    last_delta_flush = Instant::now();
                    continue;
                }
            };
            let provider_connected = matches!(
                &event,
                AgentEvent::ControllerStatus { code, .. } if code == "provider_connected"
            );
            if provider_connected {
                emit_subagent_lifecycle_event(
                    lifecycle_capture.as_ref(),
                    SubagentLifecycleEventKind::Connected,
                    serde_json::json!({ "providerConnected": true }),
                )
                .await;
                provider_invocation_index = provider_invocation_index.saturating_add(1);
                let invocation_id = format!(
                    "subagent-provider:{}:{}",
                    telemetry_identity
                        .as_ref()
                        .map(|(_, subtask_id)| subtask_id.as_str())
                        .unwrap_or("detached"),
                    provider_invocation_index
                );
                active_provider_invocation_id = Some(invocation_id.clone());
                first_provider_byte_recorded = false;
                first_visible_token_recorded = false;
                if let Some((parent_run_id, subtask_run_id)) = telemetry_identity.as_ref() {
                    let connect_ms = instant_elapsed_ms(launch_started);
                    record_subagent_launch_metric(
                        &telemetry_db,
                        parent_run_id,
                        subtask_run_id,
                        &telemetry_call_label,
                        "provider_connect_ms",
                        Some(connect_ms),
                        Some(&invocation_id),
                        if non_streaming_completion {
                            "completion_boundary"
                        } else {
                            "measured"
                        },
                    );
                    record_subtask_event(
                        &telemetry_db,
                        parent_run_id,
                        &format!("Subagent connected to provider: {telemetry_call_label}"),
                        "connected",
                        Some(&serde_json::json!({
                            "subtaskRunId": subtask_run_id,
                            "callLabel": &telemetry_call_label,
                            "connectMs": connect_ms,
                        })),
                    );
                }
            }
            if event_marks_provider_response_byte(&event) && active_provider_invocation_id.is_some()
            {
                if !first_provider_byte_recorded {
                    first_provider_byte_recorded = true;
                    if let (Some((parent_run_id, subtask_run_id)), Some(provider_invocation_id)) = (
                        telemetry_identity.as_ref(),
                        active_provider_invocation_id.as_deref(),
                    ) {
                        let elapsed_ms = instant_elapsed_ms(launch_started);
                        record_subagent_launch_metric(
                            &telemetry_db,
                            parent_run_id,
                            subtask_run_id,
                            &telemetry_call_label,
                            "first_sse_byte_ms",
                            (!non_streaming_completion).then_some(elapsed_ms),
                            Some(provider_invocation_id),
                            if non_streaming_completion {
                                "not_applicable_completion_mode"
                            } else {
                                "measured"
                            },
                        );
                    }
                }
            }
            if event_has_visible_token(&event)
                && active_provider_invocation_id.is_some()
                && !first_visible_token_recorded
            {
                first_visible_token_recorded = true;
                if let (Some((parent_run_id, subtask_run_id)), Some(provider_invocation_id)) = (
                    telemetry_identity.as_ref(),
                    active_provider_invocation_id.as_deref(),
                ) {
                    let elapsed_ms = instant_elapsed_ms(launch_started);
                    record_subagent_launch_metric(
                        &telemetry_db,
                        parent_run_id,
                        subtask_run_id,
                        &telemetry_call_label,
                        "first_visible_token_ms",
                        Some(elapsed_ms),
                        Some(provider_invocation_id),
                        "measured",
                    );
                    record_subagent_launch_metric(
                        &telemetry_db,
                        parent_run_id,
                        subtask_run_id,
                        &telemetry_call_label,
                        "frontend_first_paint_ms",
                        None,
                        Some(provider_invocation_id),
                        "not_applicable_background_worker",
                    );
                    record_subtask_event(
                        &telemetry_db,
                        parent_run_id,
                        &format!("Subagent received first token: {telemetry_call_label}"),
                        "first_token",
                        Some(&serde_json::json!({
                            "subtaskRunId": subtask_run_id,
                            "callLabel": &telemetry_call_label,
                            "firstTokenMs": elapsed_ms,
                        })),
                    );
                }
            }
            let should_flush_before_event = match &event {
                AgentEvent::Thinking { .. } => !pending_output.is_empty(),
                AgentEvent::TextDelta { .. } => !pending_thinking.is_empty(),
                _ => !pending_thinking.is_empty() || !pending_output.is_empty(),
            };
            if should_flush_before_event || transcript.flush_before(&event) {
                flush_subagent_deltas(
                    lifecycle_capture.as_ref(),
                    &mut pending_thinking,
                    &mut pending_output,
                    &mut transcript,
                )
                .await;
                last_delta_flush = Instant::now();
            }
            match event {
                AgentEvent::Thinking { content } => {
                    if !content.trim().is_empty() {
                        if !canonical_thinking {
                            pending_thinking.push_str(&content);
                        }
                        capture.thinking.push(content);
                    }
                }
                AgentEvent::Status { content, tone } => {
                    if !content.trim().is_empty() {
                        let detail = serde_json::json!({
                            "phase": "status",
                            "content": content,
                            "tone": tone,
                        });
                        capture.tool_events.push(detail.clone());
                        emit_subagent_lifecycle_event(
                            lifecycle_capture.as_ref(),
                            SubagentLifecycleEventKind::Progress,
                            detail,
                        )
                        .await;
                    }
                }
                AgentEvent::ConnectionState { state } => {
                    let detail = serde_json::json!({
                        "phase": "connection",
                        "state": state,
                    });
                    capture.tool_events.push(detail.clone());
                    emit_subagent_lifecycle_event(
                        lifecycle_capture.as_ref(),
                        SubagentLifecycleEventKind::Progress,
                        detail,
                    )
                    .await;
                }
                AgentEvent::Steering { content } => {
                    if !content.trim().is_empty() {
                        capture.tool_events.push(serde_json::json!({
                            "phase": "steering",
                            "content": content,
                        }));
                        emit_subagent_lifecycle_event(
                            lifecycle_capture.as_ref(),
                            SubagentLifecycleEventKind::InputApplied,
                            serde_json::json!({
                                "bytes": content.len(),
                                "content": content,
                                "state": "applied_at_model_boundary",
                            }),
                        )
                        .await;
                    }
                }
                AgentEvent::UsageUpdate { usage_total, .. } => {
                    capture.usage_total = usage_total;
                    if !worker_token_limit_exceeded
                        && worker_actual_token_limit
                            .is_some_and(|limit| capture.usage_total.total_tokens > limit)
                    {
                        worker_token_limit_exceeded = true;
                        capture_cancel_token.cancel();
                        let _ = fatal_error_tx.send(format!(
                            "worker actual token limit exceeded: {} > {}",
                            capture.usage_total.total_tokens,
                            worker_actual_token_limit.unwrap_or_default(),
                        ));
                    }
                }
                AgentEvent::Done {
                    usage_total,
                    finish_reason,
                    ..
                } => {
                    flush_subagent_deltas(
                        lifecycle_capture.as_ref(),
                        &mut pending_thinking,
                        &mut pending_output,
                        &mut transcript,
                    )
                    .await;
                    capture.usage_total = usage_total;
                    if !worker_token_limit_exceeded
                        && worker_actual_token_limit
                            .is_some_and(|limit| capture.usage_total.total_tokens > limit)
                    {
                        worker_token_limit_exceeded = true;
                        capture_cancel_token.cancel();
                        let _ = fatal_error_tx.send(format!(
                            "worker actual token limit exceeded: {} > {}",
                            capture.usage_total.total_tokens,
                            worker_actual_token_limit.unwrap_or_default(),
                        ));
                    }
                    capture.finish_reason = finish_reason;
                }
                AgentEvent::Error { message } => {
                    flush_subagent_deltas(
                        lifecycle_capture.as_ref(),
                        &mut pending_thinking,
                        &mut pending_output,
                        &mut transcript,
                    )
                    .await;
                    capture.error_message = Some(message.clone());
                    capture.tool_events.push(serde_json::json!({
                        "phase": "error",
                        "message": &message,
                    }));
                    let _ = fatal_error_tx.send(message);
                    capture_cancel_token.cancel();
                    break;
                }
                AgentEvent::TextDelta { delta } => {
                    if !canonical_output {
                        pending_output.push_str(&delta);
                    }
                }
                AgentEvent::ToolRunStarted { run } => {
                    transcript.boundary();
                    let detail = serde_json::json!({ "phase": "runStarted", "run": run });
                    capture.tool_events.push(detail.clone());
                    emit_subagent_lifecycle_event(
                        lifecycle_capture.as_ref(),
                        SubagentLifecycleEventKind::ToolStarted,
                        detail,
                    )
                    .await;
                }
                AgentEvent::ToolRunUpdated { run } => {
                    let detail = serde_json::json!({ "phase": "runUpdated", "run": run });
                    capture.tool_events.push(detail.clone());
                    emit_subagent_lifecycle_event(
                        lifecycle_capture.as_ref(),
                        SubagentLifecycleEventKind::Progress,
                        detail,
                    )
                    .await;
                }
                AgentEvent::ToolRunCompleted { run } => {
                    let detail = serde_json::json!({ "phase": "runCompleted", "run": run });
                    capture.tool_events.push(detail.clone());
                    emit_subagent_lifecycle_event(
                        lifecycle_capture.as_ref(),
                        SubagentLifecycleEventKind::Progress,
                        detail,
                    )
                    .await;
                }
                AgentEvent::ControllerStatus {
                    code,
                    content,
                    tone,
                } if code != "provider_connected" => {
                    emit_subagent_lifecycle_event(
                        lifecycle_capture.as_ref(),
                        SubagentLifecycleEventKind::Progress,
                        serde_json::json!({
                            "phase": "controller",
                            "code": code,
                            "content": content,
                            "tone": tone,
                        }),
                    )
                    .await;
                }
                event @ (AgentEvent::StreamBlockDelta { .. }
                | AgentEvent::StreamBlockSnapshot { .. }) => {
                    match &event {
                        AgentEvent::StreamBlockDelta { channel, .. }
                        | AgentEvent::StreamBlockSnapshot { channel, .. } => match channel {
                            nexa_core::agent::StreamBlockChannel::Thinking => {
                                canonical_thinking = true
                            }
                            nexa_core::agent::StreamBlockChannel::Answer => canonical_output = true,
                        },
                        _ => unreachable!(),
                    }
                    transcript.note_canonical(event);
                }
                event @ (AgentEvent::StreamReset { .. }
                | AgentEvent::AutoCompacted { .. }
                | AgentEvent::ApprovalRequested { .. }
                | AgentEvent::ApprovalResolved { .. }
                | AgentEvent::PlanUpdated { .. }) => {
                    if matches!(event, AgentEvent::StreamReset { .. }) {
                        transcript.boundary();
                    }
                    emit_subagent_lifecycle_event(
                        lifecycle_capture.as_ref(),
                        SubagentLifecycleEventKind::Stream,
                        serde_json::json!({ "event": event }),
                    )
                    .await;
                }
                AgentEvent::ToolCallPreparing { .. }
                | AgentEvent::ToolCallArgsDelta { .. }
                | AgentEvent::ToolCallStart { .. }
                | AgentEvent::ToolCallProgress { .. }
                | AgentEvent::ToolCallResult { .. }
                | AgentEvent::ControllerStatus { .. } => {}
            }
            if last_delta_flush.elapsed() >= Duration::from_millis(100)
                && (!pending_thinking.is_empty()
                    || !pending_output.is_empty()
                    || transcript.has_pending())
            {
                flush_subagent_deltas(
                    lifecycle_capture.as_ref(),
                    &mut pending_thinking,
                    &mut pending_output,
                    &mut transcript,
                )
                .await;
                last_delta_flush = Instant::now();
            }
        }
        capture
    }
}
