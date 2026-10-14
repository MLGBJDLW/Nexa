//! Core error types for the nexa-core crate.

use thiserror::Error;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRecoveryRequirement {
    pub action_reconciliation: bool,
    pub message: String,
}

impl CoreError {
    pub fn recovery_requirement(&self) -> Option<AgentRecoveryRequirement> {
        match self {
            Self::RecoveryRequired {
                message,
                action_reconciliation,
            } => Some(AgentRecoveryRequirement {
                action_reconciliation: *action_reconciliation,
                message: crate::sensitive_data::sanitize_diagnostic(message, None),
            }),
            _ => None,
        }
    }
}

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Embedding error: {0}")]
    Embedding(String),

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("LLM rate limited, retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("LLM context window exceeded: {0} tokens > {1} max")]
    ContextOverflow(u32, u32),

    #[error("Agent error: {0}")]
    Agent(String),

    #[error("OCR error: {0}")]
    Ocr(String),

    #[error("Video error: {0}")]
    Video(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("LLM transient error (retriable): {0}")]
    TransientLlm(String),

    /// An HTTP response was received from the provider. This is not evidence
    /// of a client/VPN connection failure even when retrying is appropriate.
    #[error("Provider HTTP {status}: {message}")]
    ProviderUnavailable { status: u16, message: String },

    #[error("Stream interrupted: {0}")]
    StreamIncomplete(String),

    #[error("Operation cancelled: {0}")]
    Cancelled(String),

    /// The current turn intentionally yielded after creating a durable
    /// interaction request. This is a non-terminal control-flow outcome: the
    /// host must keep the original turn/run resumable instead of reporting a
    /// failure or synthesizing an assistant answer.
    #[error("Agent is awaiting user input for interaction {interaction_id}")]
    AwaitingUserInput { interaction_id: String },

    /// Transient execution cannot safely continue automatically. The host must
    /// commit a resumable checkpoint through its existing Run Event outbox.
    #[error("Agent connection interrupted; resume is available: {message}")]
    RecoveryRequired {
        message: String,
        action_reconciliation: bool,
    },

    #[error("MCP error: {0}")]
    Mcp(String),

    #[error("MCP JSON-RPC error {code}: {message}")]
    McpRpc { code: i64, message: String },

    #[error("MCP authorization {code}: {message}")]
    McpAuth { code: String, message: String },

    #[error("MCP HTTP {status}: authorization rejected; sign in from settings. The operation was not replayed.")]
    McpHttpAuth {
        status: u16,
        challenge: Option<crate::mcp::oauth::BearerChallenge>,
    },

    /// An MCP failure that means the underlying connection can no longer be
    /// trusted (for example, a timeout, closed stream, or transient HTTP 5xx).
    /// Keeping this distinct from JSON-RPC/application errors prevents a tool
    /// mistake from needlessly restarting a healthy stateful MCP server.
    #[error("MCP transport error: {0}")]
    McpTransport(String),
}
