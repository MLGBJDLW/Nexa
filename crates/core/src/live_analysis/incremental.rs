use super::{Frame, Input, LiveOptions, LivePhase, Session};
use crate::llm::{CompletionRequest, ContentPart, LlmProvider, Message, ProviderStreamEvent, Role};
use futures::StreamExt;
use std::{
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};

pub(super) async fn run(
    session: &Session,
    provider: Arc<dyn LlmProvider>,
    mut request: CompletionRequest,
    options: LiveOptions,
    _input: mpsc::Receiver<Input>,
    mut frames: watch::Receiver<Option<Frame>>,
) -> Result<(), String> {
    // Live observation never inherits the agent's tool surface or side effects.
    request.tools = None;
    request.messages.clear();
    let mut ticker = tokio::time::interval(options.interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut frame_sequence = 0;
    let mut transcript_revision = 0;
    let mut last_observation = String::new();
    session.phase(LivePhase::Listening, None);
    loop {
        ticker.tick().await;
        if session.expired() {
            return Ok(());
        }
        let frame = frames.borrow_and_update().clone();
        let frame_changed = frame
            .as_ref()
            .is_some_and(|frame| frame.sequence > frame_sequence);
        let current_revision = session.transcript_revision.load(Ordering::Acquire);
        if !frame_changed && current_revision == transcript_revision {
            continue;
        }
        transcript_revision = current_revision;
        let snapshot = session.snapshot();
        let mut recent_speech = snapshot
            .entries
            .iter()
            .rev()
            .filter(|entry| entry.role == "user")
            .take(6)
            .map(|entry| entry.text.as_str())
            .collect::<Vec<_>>();
        recent_speech.reverse();
        let speech = recent_speech.join("\n");
        let mut user = Message::text(Role::User, format!("Recent speech (may be incomplete):\n{}\nPrevious observation:\n{}\nDescribe only the useful changes visible or audible now.", speech.chars().take(8_000).collect::<String>(), last_observation));
        if let Some(frame) = frame {
            user.parts.push(ContentPart::Image {
                media_type: frame.mime_type.clone(),
                data: frame.data.clone(),
            });
            if frame_changed {
                session.frame_sent(&frame, &mut frame_sequence);
            }
        }
        request.messages = vec![
            Message::text(
                Role::System,
                super::observation_instructions(&options.purpose),
            ),
            user,
        ];
        let id = format!("observation:{}", uuid::Uuid::new_v4());
        let started = Instant::now();
        session.phase(LivePhase::Analyzing, None);
        let mut stream =
            tokio::time::timeout(Duration::from_secs(20), provider.stream_events(&request))
                .await
                .map_err(|_| "Live observation could not connect to the model")?
                .map_err(|error| error.to_string())?;
        let mut output = String::new();
        let mut completed = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let event = tokio::time::timeout_at(deadline, stream.next())
                .await
                .map_err(|_| "Live observation stopped receiving model output")?;
            let Some(event) = event else {
                break;
            };
            match event {
                ProviderStreamEvent::Chunk { chunk } => {
                    if !chunk.delta.is_empty() {
                        let remaining =
                            super::MAX_ENTRY_CHARS.saturating_sub(output.chars().count());
                        output.extend(chunk.delta.chars().take(remaining));
                        session.entry(&id, "assistant", &output, false, false);
                    }
                    if let Some(reason) = chunk.finish_reason {
                        completed = reason == crate::llm::FinishReason::Stop;
                    }
                }
                ProviderStreamEvent::RecoverableError { message, .. }
                | ProviderStreamEvent::Cancelled { message } => return Err(message),
                ProviderStreamEvent::TerminalError { failure } => {
                    return Err(format!("Live observation failed: {failure:?}"))
                }
                ProviderStreamEvent::HostedTool { .. } => {
                    return Err("Live observation cannot execute tools".into())
                }
                ProviderStreamEvent::ReplayState { .. } => {}
            }
        }
        session.entry(&id, "assistant", &output, false, completed);
        last_observation = output.chars().take(2_000).collect();
        session.response_time(started);
        session.phase(LivePhase::Listening, None);
    }
}
