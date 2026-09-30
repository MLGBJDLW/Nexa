//! Persist native-agent messages without borrowing Nexa's tool workflow state.
use nexa_core::{
    agent::{AgentSteeringMessage, ExternalToolSession, PersistedAssistantMessage},
    conversation::{memory::estimate_tokens_for_model, ConversationMessage},
    db::Database,
    error::CoreError,
    llm::{Message, Role},
};
use std::sync::Arc;
use tokio::sync::Mutex;

pub(super) enum Transcript {
    NexaTools(Arc<ExternalToolSession>),
    Native {
        db: Arc<Database>,
        conversation: String,
        turn: String,
        model: String,
        order: Mutex<i64>,
    },
}

impl Transcript {
    pub(super) async fn persist_intermediate(
        &self,
        text: &str,
    ) -> Result<PersistedAssistantMessage, CoreError> {
        match self {
            Self::NexaTools(tools) => tools.persist_intermediate(text).await,
            Self::Native { .. } => Ok(PersistedAssistantMessage {
                id: self.save(Role::Assistant, text, None).await?,
                message: Message::text(Role::Assistant, text),
            }),
        }
    }
    pub(super) fn nexa_tools(&self) -> &Arc<ExternalToolSession> {
        match self {
            Self::NexaTools(tools) => tools,
            Self::Native { .. } => panic!("ACP native tools are not dispatched by Nexa"),
        }
    }

    pub(super) async fn persist_answer(
        &self,
        text: &str,
    ) -> Result<PersistedAssistantMessage, CoreError> {
        if let Self::NexaTools(tools) = self {
            return tools.persist_answer(text).await;
        }
        let id = self.save(Role::Assistant, text, None).await?;
        Ok(PersistedAssistantMessage {
            id,
            message: Message::text(Role::Assistant, text),
        })
    }

    pub(super) async fn persist_steering(
        &self,
        message: &AgentSteeringMessage,
    ) -> Result<(), CoreError> {
        if let Self::NexaTools(tools) = self {
            return tools.persist_steering(message).await;
        }
        self.save(
            Role::User,
            &message.content,
            message.image_attachments.clone(),
        )
        .await
        .map(|_| ())
    }

    async fn save(
        &self,
        role: Role,
        text: &str,
        images: Option<Vec<nexa_core::conversation::ImageAttachment>>,
    ) -> Result<String, CoreError> {
        let Self::Native {
            db,
            conversation,
            turn,
            model,
            order,
        } = self
        else {
            unreachable!()
        };
        let mut order = order.lock().await;
        let id = uuid::Uuid::new_v4().to_string();
        db.add_message(&ConversationMessage {
            id: id.clone(),
            conversation_id: conversation.clone(),
            role,
            content: text.into(),
            tool_call_id: None,
            tool_calls: vec![],
            artifacts: Some(serde_json::json!({"turnId":turn,"runtime":"external_acp"})),
            // Context sizing only; these estimates never become billable usage.
            token_count: estimate_tokens_for_model(model, text),
            created_at: String::new(),
            sort_order: *order,
            thinking: None,
            image_attachments: images,
        })?;
        *order += 1;
        Ok(id)
    }
}
