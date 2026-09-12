//! The remote surface is an explicit allowlist, independent of desktop IPC.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize)]
#[serde(
    tag = "method",
    content = "params",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RemoteCommand {
    #[serde(rename = "connection.certificate")]
    Certificate,
    #[serde(rename = "connection.preferences")]
    Preferences,
    #[serde(rename = "appearance.background")]
    ThemeBackground { asset_id: String },
    #[serde(rename = "files.preview")]
    FilePreview { path: String },
    #[serde(rename = "files.data")]
    FileData { path: String },
    #[serde(rename = "preview.html")]
    PreviewHtml { html: String },
    #[serde(rename = "evidence.get")]
    Evidence { chunk_id: String },
    #[serde(rename = "voice.start")]
    VoiceStart { request_id: String },
    #[serde(rename = "voice.audio")]
    VoiceAudio { session_id: String, data: String },
    #[serde(rename = "voice.snapshot")]
    VoiceSnapshot { session_id: String },
    #[serde(rename = "voice.finish")]
    VoiceFinish { session_id: String },
    #[serde(rename = "voice.cancel")]
    VoiceCancel { session_id: String },
    #[serde(rename = "connections.list")]
    Connections,
    #[serde(rename = "models.list")]
    Models { connection_id: String },
    #[serde(rename = "chat.list")]
    Conversations { before: Option<String> },
    #[serde(rename = "chat.read")]
    Conversation {
        conversation_id: String,
        before_order: Option<i64>,
    },
    #[serde(rename = "chat.message")]
    Message {
        conversation_id: String,
        message_id: String,
        offset: u32,
    },
    #[serde(rename = "chat.create")]
    CreateConversation {
        connection_id: String,
        model_selection: Option<Value>,
    },
    #[serde(rename = "chat.start")]
    StartTurn {
        conversation_id: String,
        connection_id: String,
        message: String,
        idempotency_key: String,
        model_selection: Option<Value>,
        #[serde(default)]
        attachments: Vec<Value>,
        execution_mode: Option<String>,
        power_mode: Option<String>,
        collaboration_mode: Option<String>,
        vision_turn_override: Option<Value>,
    },
    #[serde(rename = "chat.stop")]
    StopTurn { conversation_id: String },
    #[serde(rename = "chat.resume")]
    Resume {
        conversation_id: String,
        run_id: Option<String>,
        after_sequence: Option<u64>,
        durable_high_water: Option<u64>,
    },
    #[serde(rename = "interactions.list")]
    Interactions { conversation_id: Option<String> },
    #[serde(rename = "interactions.respond")]
    RespondInteraction { input: Value },
    #[serde(rename = "approvals.list")]
    Approvals,
    #[serde(rename = "approvals.respond")]
    RespondApproval {
        request_id: String,
        decision: String,
    },
    #[serde(rename = "live.connections")]
    LiveConnections,
    #[serde(rename = "live.start")]
    LiveStart { request: Value },
    #[serde(rename = "live.snapshot")]
    LiveSnapshot { session_id: String },
    #[serde(rename = "live.frame")]
    LiveFrame {
        session_id: String,
        mime_type: String,
        data: String,
    },
    #[serde(rename = "live.audio")]
    LiveAudio { session_id: String, data: String },
    #[serde(rename = "live.stop")]
    LiveStop { session_id: String },
    #[serde(rename = "live.summarize")]
    LiveSummarize {
        session_id: String,
        connection_id: String,
        model_selection: Option<Value>,
    },
    #[serde(rename = "live.list")]
    LiveRecords,
    #[serde(rename = "live.load")]
    LiveRecord { session_id: String },
}

impl RemoteCommand {
    pub fn validate(&self) -> Result<(), String> {
        if let Self::StartTurn {
            message,
            idempotency_key,
            attachments,
            ..
        } = self
        {
            if (message.trim().is_empty() && attachments.is_empty())
                || message.chars().count() > 64_000
            {
                return Err("Chat messages must contain 1 to 64000 characters".into());
            }
            if attachments.len() > 8
                || attachments
                    .iter()
                    .map(|item| item["base64Data"].as_str().map_or(0, str::len))
                    .sum::<usize>()
                    > 11_200_000
            {
                return Err("Attachments exceed the remote limit (8 files, 8 MiB total)".into());
            }
            uuid::Uuid::parse_str(idempotency_key)
                .map_err(|_| "A stable request ID is required for reconnection")?;
        }
        if let Self::RespondApproval { decision, .. } = self {
            if !matches!(decision.as_str(), "allow_once" | "deny") {
                return Err("Remote approvals must be explicit for this action".into());
            }
        }
        if let Self::LiveAudio { data, .. } | Self::VoiceAudio { data, .. } = self {
            if data.len() > 44_000 {
                return Err("Audio packet is too large".into());
            }
        }
        Ok(())
    }
    pub fn is_audio(&self) -> bool {
        matches!(self, Self::LiveAudio { .. } | Self::VoiceAudio { .. })
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub url: String,
    pub kind: EndpointKind,
}
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EndpointKind {
    Lan,
    Tunnel,
    Ssh,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEvent {
    pub event: String,
    pub payload: Value,
    #[serde(skip)]
    pub owner: Option<String>,
}

#[derive(Deserialize)]
pub struct RpcRequest {
    pub id: Option<u64>,
    #[serde(flatten)]
    pub command: RemoteCommand,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionManifest {
    pub server_id: String,
    pub endpoints: Vec<Endpoint>,
    pub reconnect_grace_seconds: u64,
}

#[derive(Clone)]
pub struct WebAsset {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}
