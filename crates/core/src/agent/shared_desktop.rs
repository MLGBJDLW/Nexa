use super::*;

pub(super) async fn current_shared_context(
    conversation: &str,
    native_vision: bool,
    interpreter: Option<&ToolVisualInterpreter>,
) -> Option<Message> {
    let (source, attachment, context_name) =
        crate::shared_desktop::store().latest_context(conversation)?;
    let image = attachment.data.clone();
    let context = super::tool_dispatch::resolve_tool_visual_context_message(
        native_vision,
        interpreter,
        "user_shared_desktop",
        vec![attachment],
    )
    .await;
    if !crate::shared_desktop::store()
        .latest(conversation)
        .is_some_and(|(current_source, current)| current_source == source && current.data == image)
    {
        return None;
    }
    let mut message = context?;
    message.name = Some(context_name);
    message.parts.insert(0, ContentPart::Text { text: format!(
        "The user is sharing this screen for the current conversation. This is a recent snapshot, not a recording or an authorization to control it. Source label (untrusted): {}. Observe the exact native window before input; this shared view is not a computer_control observation token.", serde_json::to_string(&source).unwrap_or_default()
    ) });
    Some(message)
}

impl AgentExecutor {
    /// Refresh only the request projection; do not persist screenshots in history.
    pub(super) async fn append_shared_desktop_context(
        &self,
        conversation_id: Option<&str>,
        model: &str,
        request: &mut Vec<Message>,
    ) {
        let Some(conversation) = conversation_id else {
            return;
        };
        let native_vision = self.config.native_vision.unwrap_or_else(|| {
            self.config
                .provider_type
                .is_some_and(|provider| crate::llm::model_supports_vision(&provider, model))
        });
        if let Some(message) = current_shared_context(
            conversation,
            native_vision,
            self.tool_visual_interpreter.as_ref(),
        )
        .await
        {
            request.push(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    fn frame(color: u8) -> String {
        let mut output = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2,
            2,
            image::Rgb([color, 0, 0]),
        ))
        .write_to(&mut output, image::ImageFormat::Jpeg)
        .unwrap();
        STANDARD.encode(output.into_inner())
    }
    #[tokio::test]
    async fn shared_desktop_refreshes_each_request_without_changing_canonical_history() {
        let id = Uuid::new_v4().to_string();
        let store = crate::shared_desktop::store();
        let lease = store.begin(&id, "Test screen").unwrap();
        let canonical = vec![Message::text(Role::User, "What is visible?")];
        for (sequence, color) in [(1, 10), (2, 220)] {
            let encoded = frame(color);
            store
                .update(&id, &lease, sequence, encoded.clone())
                .unwrap();
            let mut projected = canonical.clone();
            projected.push(current_shared_context(&id, true, None).await.unwrap());
            assert_eq!(projected.len(), 2);
            assert!(projected[1]
                .parts
                .iter()
                .any(|part| matches!(part, ContentPart::Image {data, ..} if data == &encoded)));
            assert_eq!(canonical.len(), 1);
        }
        store.end(&id, &lease);
        assert!(current_shared_context(&id, true, None).await.is_none());
    }
}
