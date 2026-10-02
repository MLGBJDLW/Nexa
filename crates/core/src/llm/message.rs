//! Immutable message snapshots with copy-on-write edits and stable analysis identity.

use std::ops::{Deref, DerefMut};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use serde::{Deserialize, Serialize};

use super::{
    provider_turn, CacheBoundaryHint, ContentPart, PromptCacheHint, PromptLifetime,
    PromptStability, Role, ToolCallRequest,
};

static NEXT_MESSAGE_REVISION: AtomicU64 = AtomicU64::new(1);

fn next_revision() -> u64 {
    let mut current = NEXT_MESSAGE_REVISION.load(Ordering::Relaxed);
    loop {
        let next = current
            .checked_add(1)
            .expect("message revision identity exhausted");
        match NEXT_MESSAGE_REVISION.compare_exchange_weak(
            current,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return current,
            Err(observed) => current = observed,
        }
    }
}

/// A message is a cheap, immutable snapshot until a caller edits its fields.
/// Read-only request, retry and replay projections share all large payloads.
#[derive(Debug, Clone)]
pub struct Message(Arc<MessageBody>);

#[derive(Debug, Clone)]
struct MessageBody {
    revision: u64,
    data: MessageData,
}

/// Serializable message fields. This retains the existing JSON contract;
/// allocation/revision identity never enters storage, IPC or provider input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MessageData {
    pub role: Role,
    pub parts: Vec<ContentPart>,
    /// Optional name for tool messages (the tool-call id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Tool calls requested by the assistant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallRequest>>,
    /// Provider-specific assistant reasoning content to pass back in
    /// multi-step tool loops (e.g. DeepSeek `reasoning_content`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    /// Internal prompt-compiler metadata. Provider adapters consume this
    /// sidecar and never include it in wire message content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_hint: Option<PromptCacheHint>,
}

impl From<MessageData> for Message {
    fn from(data: MessageData) -> Self {
        Self(Arc::new(MessageBody {
            revision: next_revision(),
            data,
        }))
    }
}

impl Deref for Message {
    type Target = MessageData;
    fn deref(&self) -> &Self::Target {
        &self.0.data
    }
}

impl DerefMut for Message {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let body = Arc::make_mut(&mut self.0);
        body.revision = next_revision();
        &mut body.data
    }
}

impl PartialEq for Message {
    fn eq(&self, other: &Self) -> bool {
        self.revision() == other.revision() || self.0.data == other.0.data
    }
}

impl Serialize for Message {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.data.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        MessageData::deserialize(deserializer).map(Self::from)
    }
}

impl Message {
    /// Stable only for this immutable snapshot. Any mutable field access gets
    /// a fresh identity, even when it can edit a uniquely owned allocation.
    pub(crate) fn revision(&self) -> u64 {
        self.0.revision
    }

    /// Consume a snapshot when an owner genuinely needs its individual fields.
    /// Shared read-only consumers should borrow fields instead.
    pub fn into_data(self) -> MessageData {
        Arc::try_unwrap(self.0)
            .map(|body| body.data)
            .unwrap_or_else(|body| body.data.clone())
    }
}

impl Message {
    /// Create a text-only message.
    pub fn text(role: Role, content: impl Into<String>) -> Self {
        Self::from(MessageData {
            role,
            parts: vec![ContentPart::Text {
                text: content.into(),
            }],
            name: None,
            tool_calls: None,
            reasoning_content: None,
            prompt_cache_hint: None,
        })
    }

    /// Create a text message with a name.
    pub fn text_with_name(role: Role, content: impl Into<String>, name: impl Into<String>) -> Self {
        Self::from(MessageData {
            role,
            parts: vec![ContentPart::Text {
                text: content.into(),
            }],
            name: Some(name.into()),
            tool_calls: None,
            reasoning_content: None,
            prompt_cache_hint: None,
        })
    }

    pub fn with_prompt_cache_hint(
        mut self,
        stability: PromptStability,
        boundary: CacheBoundaryHint,
    ) -> Self {
        let hint = Some(PromptCacheHint {
            stability,
            boundary,
            lifetime: PromptLifetime::Turn,
        });
        if self.prompt_cache_hint != hint {
            self.prompt_cache_hint = hint;
        }
        self
    }

    pub fn with_prompt_lifetime(mut self, lifetime: PromptLifetime) -> Self {
        if self
            .prompt_cache_hint
            .is_some_and(|hint| hint.lifetime != lifetime)
        {
            if let Some(hint) = self.prompt_cache_hint.as_mut() {
                hint.lifetime = lifetime;
            }
        }
        self
    }

    pub fn prompt_cache_hint(&self) -> Option<(PromptStability, CacheBoundaryHint)> {
        self.prompt_cache_hint
            .map(|hint| (hint.stability, hint.boundary))
    }

    pub fn prompt_lifetime(&self) -> PromptLifetime {
        self.prompt_cache_hint
            .map(|hint| hint.lifetime)
            .unwrap_or_default()
    }

    /// Get the combined text content from all text parts.
    pub fn text_content(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Check if this message has any image parts.
    pub fn has_images(&self) -> bool {
        self.parts
            .iter()
            .any(|p| matches!(p, ContentPart::Image { .. }))
    }

    /// Get all image parts.
    pub fn image_parts(&self) -> Vec<&ContentPart> {
        self.parts
            .iter()
            .filter(|p| matches!(p, ContentPart::Image { .. }))
            .collect()
    }

    pub fn provider_turn(&self) -> Option<&provider_turn::ProviderTurnEnvelope> {
        self.parts.iter().find_map(|part| match part {
            ContentPart::ProviderTurn { envelope } => Some(envelope.as_ref()),
            _ => None,
        })
    }

    pub fn set_provider_turn(&mut self, envelope: provider_turn::ProviderTurnEnvelope) {
        self.clear_provider_turn();
        self.parts.push(ContentPart::ProviderTurn {
            envelope: Box::new(envelope),
        });
    }

    /// Remove provider-native replay state when a history repair changes the
    /// assistant/tool envelope it authenticated.
    pub fn clear_provider_turn(&mut self) {
        if self.provider_turn().is_some() {
            self.parts
                .retain(|part| !matches!(part, ContentPart::ProviderTurn { .. }));
        }
        if self
            .tool_calls
            .as_ref()
            .is_some_and(|calls| calls.iter().any(|call| call.thought_signature.is_some()))
        {
            if let Some(tool_calls) = self.tool_calls.as_mut() {
                for tool_call in tool_calls {
                    tool_call.thought_signature = None;
                }
            }
        }
    }

    /// Remove secret-adjacent provider replay state before serializing a
    /// message to the desktop UI or another display-only consumer.
    pub fn without_provider_turn(mut self) -> Self {
        self.clear_provider_turn();
        self
    }
}
