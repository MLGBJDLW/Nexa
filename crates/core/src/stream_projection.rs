//! Canonical output block identity and UTF-8 offsets shared by every host.
use crate::agent::{AgentEvent, StreamBlockChannel};
use crate::agent_run::AgentRunEvent;
use crate::run_event_outbox::{AgentRunEventOutbox, AgentRunEventSubmitError};
use uuid::Uuid;

#[derive(Default)]
pub struct StreamBlockProjector {
    answer: Option<(String, usize)>,
    thinking: Option<(String, usize)>,
}
impl StreamBlockProjector {
    pub fn rotate(&mut self) {
        self.answer = None;
        self.thinking = None;
    }
    pub fn submit(
        &mut self,
        outbox: &AgentRunEventOutbox,
        run_id: &str,
        turn_id: Option<&str>,
        channel: StreamBlockChannel,
        text: &str,
    ) -> Result<(), AgentRunEventSubmitError> {
        let block = match channel {
            StreamBlockChannel::Answer => &mut self.answer,
            StreamBlockChannel::Thinking => &mut self.thinking,
        };
        let (id, offset) = block.get_or_insert_with(|| (format!("stream-{}", Uuid::new_v4()), 0));
        for chunk in split_text_by_utf8_bytes(text, 8 * 1024) {
            outbox.submit(AgentRunEvent::output_delta(
                run_id, turn_id, 0, id, channel, *offset, chunk,
            ))?;
            *offset += chunk.len();
        }
        Ok(())
    }
}

pub fn split_text_by_utf8_bytes(text: &str, max_bytes: usize) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let max_bytes = max_bytes.max(1);
    if text.len() <= max_bytes {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        let mut end = (start + max_bytes).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = start
                + text[start..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(1);
        }
        chunks.push(&text[start..end]);
        start = end;
    }
    chunks
}

pub fn agent_event_rotates_stream_blocks(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::StreamReset { .. }
            | AgentEvent::ToolRunStarted { .. }
            | AgentEvent::ToolRunCompleted { .. }
    )
}
