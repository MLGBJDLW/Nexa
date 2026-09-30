//! Native ACP agents own their tools and authentication. No API key is borrowed,
//! no reported tool is executed twice, and uncertain prompts are never replayed.
mod catalog;
pub(super) mod client;
mod config;
mod mcp;
mod pool;
mod projection;
mod terminals;
#[cfg(test)]
mod tests;
mod transport;

use super::{AgentRuntimeTurnRequest, PreparedTurn};
pub(crate) use catalog::Model;
use catalog::Session;
use futures::{stream::FuturesUnordered, StreamExt};
use nexa_core::{
    agent::{AgentEvent, AgentExecutionMode, StreamBlockChannel},
    approval::{ApprovalRequest, ApprovalRisk, ToolPermissionKey},
    error::CoreError,
    external_agent::{preset, ExternalAgentLaunch},
    llm::Message,
};
pub(crate) use pool::shutdown;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use transport::{rpc_result, Wire};

type Result<T> = std::result::Result<T, CoreError>;
fn error(cause: impl std::fmt::Display) -> CoreError {
    CoreError::Agent(format!("External agent: {cause}"))
}

pub(crate) async fn probe(
    provider: &str,
    launch: &ExternalAgentLaunch,
    db: &nexa_core::db::Database,
    model: Option<&str>,
) -> Result<Catalog> {
    let preset = preset(provider).ok_or_else(|| error("Unknown external agent"))?;
    let servers = mcp::selected(db, &launch.mcp_server_ids)?;
    let mut wire = Wire::start(preset, launch)?;
    tokio::time::timeout(std::time::Duration::from_secs(45), async {
        let mut session =
            Session::connect_with_mcp(&mut wire, &launch.working_directory, &servers).await?;
        session
            .configure(&mut wire, model, &launch.config_options, None)
            .await?;
        Ok(Catalog {
            models: session.models,
            config_options: session.config_options,
            commands: session.commands,
        })
    })
    .await
    .map_err(|_| error("External agent connection check timed out"))?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Catalog {
    pub models: Vec<catalog::Model>,
    pub config_options: Vec<config::ConfigOption>,
    pub commands: Vec<String>,
}

pub(crate) async fn run(provider: &str, request: AgentRuntimeTurnRequest) -> Result<Message> {
    if request.cancellation.is_cancelled() {
        return Err(CoreError::Cancelled("Stopped by user".into()));
    }
    // ACP plan modes can permit writes and are not Nexa's read-only policy.
    if request.config.execution_mode == AgentExecutionMode::Plan {
        return Err(error("Nexa Plan mode is unavailable for native ACP agents. Use a direct API agent for a read-only plan."));
    }
    let binding = request
        .external
        .as_ref()
        .ok_or_else(|| error("Select an external-agent profile before starting a turn"))?
        .clone();
    let servers = mcp::selected(&request.db, &binding.launch.mcp_server_ids)?;
    let key = blake3::hash(
        json!([
            provider,
            binding.profile_id,
            binding.launch,
            request.conversation_id,
            request.dependencies.tools.workspace(),
            request.db.load_privacy_config()?,
            servers
        ])
        .to_string()
        .as_bytes(),
    )
    .to_string();
    let history = history_fingerprint(
        &request.db,
        &request.conversation_id,
        Some(request.next_sort_order - 1),
    )?;
    let db = request.db.clone();
    let conversation = request.conversation_id.clone();
    let mut connection = match pool::take(&key, &history) {
        Some(connection) => {
            stage(&request.events, "reusing").await?;
            connection
        }
        None => {
            stage(&request.events, "starting").await?;
            let mut wire = Wire::start(
                preset(provider).ok_or_else(|| error("Unknown external agent"))?,
                &binding.launch,
            )?;
            stage(&request.events, "connecting").await?;
            let session = tokio::select! {
                _ = request.cancellation.cancelled() => return Err(CoreError::Cancelled("Stopped while connecting".into())),
                result = Session::connect_with_mcp(&mut wire, &binding.launch.working_directory, &servers) => result?,
            };
            pool::Connected {
                wire,
                session,
                history: String::new(),
                context: String::new(),
            }
        }
    };
    let answer = run_initialized(provider, request, &mut connection).await?;
    // A changed/rewound transcript, different profile or workspace never reuses
    // hidden upstream state. Only a completed, persisted turn is cached.
    if let Ok(history) = history_fingerprint(&db, &conversation, None) {
        connection.history = history;
        pool::put(key, connection);
    }
    Ok(answer)
}

fn history_fingerprint(
    db: &nexa_core::db::Database,
    conversation: &str,
    before: Option<i64>,
) -> Result<String> {
    let history = db
        .get_messages(conversation)?
        .into_iter()
        .filter(|message| before.is_none_or(|before| message.sort_order < before))
        .map(|message| {
            json!([
                message.id,
                message.role,
                message.content,
                message.image_attachments
            ])
        })
        .collect::<Vec<_>>();
    Ok(blake3::hash(serde_json::to_vec(&history)?.as_slice()).to_string())
}

async fn stage(events: &tokio::sync::mpsc::Sender<AgentEvent>, value: &str) -> Result<()> {
    events
        .send(AgentEvent::ControllerStatus {
            code: format!("external_agent_{value}"),
            content: format!("External agent: {value}"),
            tone: None,
        })
        .await
        .map_err(error)
}

#[cfg(test)]
async fn run_connected(
    provider: &str,
    request: AgentRuntimeTurnRequest,
    mut wire: Wire,
    cwd: &str,
) -> Result<Message> {
    let cancellation = request.cancellation.clone();
    let session = tokio::select! {
        _ = cancellation.cancelled() => return Err(CoreError::Cancelled("Stopped while connecting".into())),
        result = Session::connect(&mut wire, cwd) => result?,
    };
    let mut connection = pool::Connected {
        wire,
        session,
        history: String::new(),
        context: String::new(),
    };
    run_initialized(provider, request, &mut connection).await
}

async fn run_initialized(
    provider: &str,
    mut request: AgentRuntimeTurnRequest,
    connection: &mut pool::Connected,
) -> Result<Message> {
    let cancellation = request.cancellation.clone();
    let model = request
        .config
        .model
        .as_deref()
        .unwrap_or(catalog::DEFAULT_MODEL);
    stage(&request.events, "model").await?;
    let preferences = request
        .external
        .as_ref()
        .map(|binding| binding.launch.config_options.clone())
        .unwrap_or_default();
    let effort = request
        .config
        .reasoning_effort
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?
        .and_then(|value| value.as_str().map(str::to_owned));
    tokio::select! {
        _ = cancellation.cancelled() => return Err(CoreError::Cancelled("Stopped while configuring the external agent".into())),
        result = connection.session.configure(&mut connection.wire, Some(model), &preferences, effort.as_deref()) => result?,
    }
    request
        .events
        .send(AgentEvent::ControllerStatus {
            code: "provider_connected".into(),
            content: "External agent connected".into(),
            tone: None,
        })
        .await
        .map_err(error)?;
    let context = json!([
        request.config.system_prompt,
        request.config.volatile_system_sections,
        request
            .dependencies
            .selected_skills
            .iter()
            .chain(&request.dependencies.auto_loaded_skills)
            .map(|skill| (&skill.id, &skill.content, skill.enabled))
            .collect::<Vec<_>>()
    ])
    .to_string();
    let warm = !connection.history.is_empty() && !connection.context.is_empty();
    if warm {
        request.history.clear();
    }
    let mut turn = request.prepare(connection.session.images)?;
    if warm && connection.context == context {
        turn.system_prompt.clear();
    }
    if native_command(&turn.prompt).is_none() {
        connection.context = context;
    }
    let mut output = super::projection::Projection::for_turn(&turn);
    let mut reports = projection::ToolReports::new(provider)?;
    let result = drive(
        provider,
        &mut turn,
        &mut connection.wire,
        &mut connection.session,
        &mut output,
        &mut reports,
    )
    .await;
    match result {
        Ok(()) => output.finish(&turn).await,
        Err(cause) => {
            let _ = reports
                .close_pending(&turn.events, turn.cancellation.is_cancelled())
                .await;
            output.persist_partial(&turn).await?;
            Err(cause)
        }
    }
}

fn permission_target(scope: &str, session: &str, id: &str, call: &Value) -> String {
    let mut action = json!({"scope":scope,"kind":call["kind"],"title":call["title"],"rawInput":call["rawInput"],"locations":call["locations"]});
    if call.get("rawInput").is_none_or(Value::is_null) {
        // Incomplete action details cannot safely support a reusable grant.
        action["unidentifiedInvocation"] = json!([session, id]);
    }
    fn sort(value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.sort_keys();
                for value in map.values_mut() {
                    sort(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    sort(value);
                }
            }
            _ => {}
        }
    }
    sort(&mut action);
    blake3::hash(action.to_string().as_bytes()).to_string()
}

fn permission(
    provider: &str,
    turn: &PreparedTurn,
    session: &str,
    params: &Value,
) -> impl std::future::Future<Output = Result<Value>> + Send + 'static {
    let provider = provider.to_string();
    let session = session.to_string();
    let params = params.clone();
    let scope = turn.permission_scope.clone();
    let events = turn.events.clone();
    let approval = turn.approval.clone();
    let cancellation = turn.cancellation.clone();
    async move {
        if params["sessionId"] != session {
            return Err(error(
                "Permission request belongs to a different ACP session",
            ));
        }
        let call = &params["toolCall"];
        let id = call["toolCallId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| error("Permission request has no tool call id"))?;
        // A generic ACP allow_always option can grant much more than this call.
        // Nexa's reusable decisions only ever select the upstream allow_once.
        let choices = params["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|option| option["kind"] == "allow_once" && option["optionId"].is_string())
            .take(64)
            .collect::<Vec<_>>();
        let allow = choices
            .first()
            .and_then(|option| option["optionId"].as_str());
        let deny = params["options"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|option| option["kind"] == "reject_once")
            .and_then(|option| option["optionId"].as_str());
        if allow.is_none() {
            return Ok(json!({"outcome":{"outcome":"cancelled"}}));
        }
        let mut request = ApprovalRequest::new(
            uuid::Uuid::new_v4().to_string(),
            format!("external_agent:{provider}"),
            call,
            ApprovalRisk::High,
            call["title"]
                .as_str()
                .unwrap_or("External agent requests permission"),
        );
        let selection = choices.len() > 1;
        if selection {
            request.choices = choices
                .iter()
                .map(|option| {
                    option["name"]
                        .as_str()
                        .unwrap_or("Select option")
                        .chars()
                        .take(500)
                        .collect()
                })
                .collect();
        }
        let target = if selection {
            blake3::hash(
                json!([scope, session, id, params["options"]])
                    .to_string()
                    .as_bytes(),
            )
            .to_string()
        } else {
            permission_target(&scope, &session, id, call)
        };
        let key = ToolPermissionKey::new(
            &request.tool_name,
            if selection {
                "external_agent_choice"
            } else {
                "external_agent_action"
            },
            target,
        );
        request.permission_key = key.permission_key();
        request.target_kind = key.target_kind;
        request.target_value = key.target_value;
        events
            .send(AgentEvent::ApprovalRequested {
                request: request.clone(),
            })
            .await
            .map_err(error)?;
        let decision = approval(request.clone()).await;
        events
            .send(AgentEvent::ApprovalResolved {
                request_id: request.id,
                decision,
            })
            .await
            .map_err(error)?;
        let selected = if selection {
            match decision {
                nexa_core::approval::ApprovalDecision::SelectOption(index) => choices
                    .get(index as usize)
                    .and_then(|option| option["optionId"].as_str()),
                _ => deny,
            }
        } else if decision.is_allowed() {
            allow
        } else {
            deny
        };
        if cancellation.is_cancelled() {
            return Ok(json!({"outcome":{"outcome":"cancelled"}}));
        }
        Ok(match selected {
            Some(id) => json!({"outcome":{"outcome":"selected","optionId":id}}),
            None => json!({"outcome":{"outcome":"cancelled"}}),
        })
    }
}

async fn drive(
    provider: &str,
    turn: &mut PreparedTurn,
    wire: &mut Wire,
    session: &mut Session,
    output: &mut super::projection::Projection,
    reports: &mut projection::ToolReports,
) -> Result<()> {
    stage(&turn.events, "waiting").await?;
    session.sync(wire)?;
    let command = native_command(&turn.prompt).map(str::to_owned);
    if command.is_some() && !turn.images.is_empty() {
        return Err(error("Native slash commands require text-only input"));
    }
    let mut prompt = vec![
        json!({"type":"text","text":if command.is_some() { turn.prompt.clone() } else { format!("{}\n\nCurrent user message:\n{}",turn.system_prompt,turn.prompt) }}),
    ];
    prompt.extend(
        turn.images
            .iter()
            .map(|(mime, data)| json!({"type":"image","mimeType":mime,"data":data})),
    );
    let mut prompt_id = wire
        .send(
            "session/prompt",
            json!({"sessionId":session.id,"prompt":prompt}),
        )
        .await?;
    let mut segment = 0u32;
    let mut answer_blocks: Vec<(String, String)> = Vec::new();
    let mut span = 0u32;
    let mut steering = VecDeque::new();
    let mut steering_closed = false;
    let mut waiting = true;
    let receipts = nexa_core::runtime_receipts::RuntimeReceipts::new()?;
    let mut service_calls: HashMap<String, String> = HashMap::new();
    type ServiceResult =
        std::pin::Pin<Box<dyn std::future::Future<Output = (Value, Result<Value>)> + Send>>;
    let mut pending_services = FuturesUnordered::<ServiceResult>::new();
    let mut terminal_tick = tokio::time::interval(std::time::Duration::from_millis(250));
    terminal_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let message = tokio::select! {
            biased;
            _ = turn.cancellation.cancelled() => {
                // Gracefully notify before process-tree teardown. Never wait on
                // an unbounded prompt response after the user has stopped it.
                let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    wire.notify("session/cancel",json!({"sessionId":session.id})).await?;
                    loop {
                        let message = wire.receive().await?;
                        if message.get("method").is_none() && message["id"].as_u64() == Some(prompt_id) { break; }
                        if message["method"] == "session/request_permission" && message.get("id").is_some() {
                            wire.write(json!({"jsonrpc":"2.0","id":message["id"],"result":{"outcome":{"outcome":"cancelled"}}})).await?;
                        } else if message["method"] == "session/update" && message["params"]["sessionId"] == session.id {
                            let update = &message["params"]["update"];
                            if matches!(update["sessionUpdate"].as_str(),Some("tool_call" | "tool_call_update")) {
                                reports.update(&turn.events, update, &session.terminals).await?;
                            }
                        }
                    }
                    Ok::<_, CoreError>(())
                }).await;
                return Err(CoreError::Cancelled("Stopped by user".into()));
            },
            incoming = turn.steering.recv(), if !steering_closed => {
                match incoming {
                    Some(item) if item.recovery_control.is_none() => {
                        if steering.len() >= 32 { return Err(error("ACP steering queue is full")); }
                        steering.push_back(item);
                    },
                    Some(_) => return Err(error("ACP does not support Nexa recovery controls")),
                    None => steering_closed = true,
                }
                continue;
            },
            Some((id, result)) = pending_services.next(), if !pending_services.is_empty() => {
                let response = match result {
                    Ok(value) => json!({"jsonrpc":"2.0","id":id,"result":value}),
                    Err(cause) => json!({"jsonrpc":"2.0","id":id,"error":client::rpc_error(&cause)}),
                };
                if let Some(fingerprint) = service_calls.remove(&id.to_string()) { receipts.insert(&id.to_string(), &fingerprint, &response)?; }
                wire.write(response).await?;
                continue;
            },
            _ = terminal_tick.tick() => {
                reports.refresh_terminals(&turn.events, &session.terminals).await?;
                continue;
            },
            message = wire.receive() => message?,
        };
        if let Some(method) = message["method"].as_str() {
            if message.get("id").is_some() {
                let key = message["id"].to_string();
                let fingerprint = blake3::hash(message.to_string().as_bytes()).to_string();
                if let Some((previous, reply)) = receipts.get::<Value>(&key)? {
                    if previous != fingerprint {
                        return Err(error("ACP request id reused with different arguments"));
                    }
                    wire.write(reply).await?;
                    continue;
                }
                if let Some(previous) = service_calls.get(&key) {
                    if previous != &fingerprint {
                        return Err(error(
                            "ACP pending request id reused with different arguments",
                        ));
                    }
                    continue;
                }
                if method == "session/request_permission" {
                    if pending_services.len() >= 64 {
                        return Err(error("ACP has too many simultaneous client requests"));
                    }
                    let permission = permission(provider, turn, &session.id, &message["params"]);
                    service_calls.insert(key, fingerprint);
                    pending_services.push(Box::pin(async move {
                        (message["id"].clone(), permission.await)
                    }));
                } else if client::is_service(method) {
                    if message["params"]["sessionId"] != session.id {
                        wire.write(json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32602,"message":"ACP client request belongs to a different session"}})).await?;
                        continue;
                    }
                    if pending_services.len() >= 64 {
                        return Err(error("ACP has too many simultaneous client requests"));
                    }
                    let files = turn.files.clone();
                    let terminals = session.terminals.clone();
                    let cwd = session.cwd.clone();
                    let method = method.to_string();
                    service_calls.insert(key.clone(), fingerprint);
                    pending_services.push(Box::pin(async move {
                        let result = files
                            .handle(
                                &format!("acp-client:{key}"),
                                &method,
                                &message["params"],
                                &cwd,
                                &terminals,
                            )
                            .await;
                        (message["id"].clone(), result)
                    }));
                } else {
                    wire.unsupported(&message).await?;
                }
                continue;
            }
            if method != "session/update" {
                continue;
            }
            let params = &message["params"];
            if params["sessionId"] != session.id {
                return Err(error("ACP update belongs to a different session"));
            }
            let update = &params["update"];
            if session.update(update)? {
                continue;
            }
            if waiting
                && matches!(
                    update["sessionUpdate"].as_str(),
                    Some(
                        "agent_message_chunk"
                            | "agent_thought_chunk"
                            | "tool_call"
                            | "tool_call_update"
                    )
                )
            {
                stage(&turn.events, "active").await?;
                waiting = false;
            }
            match update["sessionUpdate"].as_str().unwrap_or_default() {
                "agent_message_chunk" | "agent_thought_chunk" => {
                    if update["content"]["type"] != "text" {
                        return Err(error(
                            "This external agent returned unsupported non-text output",
                        ));
                    }
                    let text = update["content"]["text"]
                        .as_str()
                        .ok_or_else(|| error("Invalid ACP text chunk"))?;
                    let thought = update["sessionUpdate"] == "agent_thought_chunk";
                    let block_id = format!(
                        "acp:{segment}:{span}:{}",
                        if thought { "thought" } else { "answer" }
                    );
                    if !thought {
                        if answer_blocks.last().is_none_or(|(id, _)| id != &block_id) {
                            answer_blocks.push((block_id.clone(), String::new()));
                        }
                        answer_blocks
                            .last_mut()
                            .expect("answer block")
                            .1
                            .push_str(text);
                    }
                    output
                        .delta(
                            &turn.events,
                            &block_id,
                            if thought {
                                StreamBlockChannel::Thinking
                            } else {
                                StreamBlockChannel::Answer
                            },
                            text,
                        )
                        .await?;
                }
                "tool_call" | "tool_call_update" => {
                    // A native tool boundary settles earlier assistant progress.
                    // Keep only the active response in memory during long runs.
                    for (id, text) in &answer_blocks {
                        output.complete_block(&turn.events, id, text).await?;
                    }
                    if let Some(message) = output
                        .persist_settled(turn, answer_blocks.drain(..).map(|(id, _)| id).collect())
                        .await?
                    {
                        output.native_final_fallback = Some(message);
                    }
                    output.clear_answer();
                    reports
                        .update(&turn.events, update, &session.terminals)
                        .await?;
                    span = span.saturating_add(1);
                }
                "plan" => {
                    let entries = update["entries"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .take(64)
                        .filter_map(|entry| entry["content"].as_str())
                        .map(|content| content.chars().take(500).collect::<String>())
                        .collect::<Vec<_>>();
                    turn.events
                        .send(AgentEvent::Status {
                            content: format!("External agent plan:\n{}", entries.join("\n")),
                            tone: None,
                        })
                        .await
                        .map_err(error)?;
                }
                "usage_update" => {
                    // `used` is a context snapshot, not a billable token delta.
                    if let Some(used) = update["used"]
                        .as_u64()
                        .and_then(|used| u32::try_from(used).ok())
                    {
                        let capacity = update["size"].as_u64().and_then(|v| u32::try_from(v).ok());
                        output
                            .context_snapshot(&turn.events, used, capacity, vec![])
                            .await?;
                    }
                }
                _ => {} // Forward-compatible status/config metadata, not output.
            }
            continue;
        }
        if message["id"].as_u64() != Some(prompt_id) {
            return Err(error("Uncorrelated ACP response"));
        }
        let completed = rpc_result(message, "session/prompt")?;
        if !pending_services.is_empty() {
            return Err(error(
                "External agent completed with pending client service requests",
            ));
        }
        if !matches!(
            completed["stopReason"].as_str(),
            Some("end_turn" | "refusal")
        ) {
            return Err(error(format!(
                "External agent stopped without completing the turn ({})",
                completed["stopReason"]
            )));
        }
        if reports.has_pending() {
            return Err(error(
                "External agent completed with unfinished tool reports",
            ));
        }
        for (id, text) in &answer_blocks {
            output.complete_block(&turn.events, id, text).await?;
        }
        output.select_answer_blocks(answer_blocks.iter().map(|(id, _)| id.clone()).collect())?;
        if steering.is_empty() {
            if command.is_some() && output.answer.trim().is_empty() {
                output
                    .complete(
                        &turn.events,
                        "native-command-result",
                        &format!(
                            "External agent command completed: /{}",
                            command.as_deref().unwrap_or_default()
                        ),
                    )
                    .await?;
            }
            return Ok(());
        }
        output.persist_completed_answer(turn).await?;
        let mut next = Vec::new();
        while let Some(item) = steering.pop_front() {
            if item
                .image_attachments
                .as_ref()
                .is_some_and(|images| !images.is_empty())
            {
                return Err(error(
                    "Send images after this external-agent turn completes",
                ));
            }
            turn.transcript.persist_steering(&item).await?;
            turn.events
                .send(AgentEvent::Steering {
                    content: item.content.clone(),
                })
                .await
                .map_err(error)?;
            next.push(
                json!({"type":"text","text":super::redact_user_text(&item.content,&turn.privacy)}),
            );
        }
        segment += 1;
        answer_blocks.clear();
        span = 0;
        stage(&turn.events, "waiting").await?;
        waiting = true;
        prompt_id = wire
            .send(
                "session/prompt",
                json!({"sessionId":session.id,"prompt":next}),
            )
            .await?;
    }
}

fn native_command(prompt: &str) -> Option<&str> {
    let name = prompt.trim().strip_prefix('/')?.split_whitespace().next()?;
    (!name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    .then_some(name)
}
