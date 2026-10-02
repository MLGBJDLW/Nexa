use super::*;

fn image_bytes(message: &Message) -> &str {
    message
        .parts
        .iter()
        .find_map(|part| match part {
            ContentPart::Image { data, .. } => Some(data.as_str()),
            _ => None,
        })
        .unwrap()
}

fn image_message() -> Message {
    let mut message = Message::text(Role::User, "Inspect this image without editing it.");
    message.parts.push(ContentPart::Image {
        media_type: "image/png".into(),
        data: "large-image-payload".repeat(65_536),
    });
    message
}

#[test]
fn request_snapshots_share_unchanged_large_payloads() {
    let original = image_message();
    let snapshot = original.clone();
    let retry = snapshot.clone();
    assert_eq!(image_bytes(&original), image_bytes(&retry));
    assert_eq!(
        image_bytes(&original).as_ptr(),
        image_bytes(&snapshot).as_ptr(),
        "creating a read-only request snapshot must not allocate another image payload"
    );
    assert_eq!(
        image_bytes(&original).as_ptr(),
        image_bytes(&retry).as_ptr(),
        "retry snapshots must retain the same immutable payload"
    );
}

#[test]
fn replay_projection_reuses_unchanged_message_payloads() {
    let message = image_message();
    let original = image_bytes(&message).as_ptr();
    let messages = vec![message];
    let route = provider_turn::RouteSnapshot::unknown(
        "fixture",
        "fixture-model",
        reasoning_profile::ReasoningReplayPolicy::NotRequired,
    );
    let projection = reasoning_replay::prepare_provider_replay_history(&messages, &route);
    assert_eq!(projection.omitted_units, 0);
    assert_eq!(
        image_bytes(&projection.messages[0]).as_ptr(),
        original,
        "a no-op replay projection must not copy text or image bodies"
    );
}

#[test]
fn changing_a_snapshot_does_not_change_its_source() {
    let original = image_message();
    let mut snapshot = original.clone();
    assert_eq!(snapshot.revision(), original.revision());
    let ContentPart::Text { text } = &mut snapshot.parts[0] else {
        panic!("text fixture");
    };
    text.push_str(" Changed only in the request projection.");
    snapshot.name = Some("projection-only".into());
    assert_eq!(
        original.text_content(),
        "Inspect this image without editing it."
    );
    assert!(original.name.is_none());
    assert!(snapshot.text_content().contains("projection"));
    assert_eq!(image_bytes(&original), image_bytes(&snapshot));
    assert_ne!(snapshot.revision(), original.revision());
}

#[test]
fn message_snapshot_serialization_preserves_the_public_message_shape() {
    let expected = serde_json::json!({
        "role": "assistant",
        "parts": [{"type": "text", "text": "Checked the file."}],
        "toolCalls": [{"id": "call-1", "name": "read_file", "arguments": "{}"}],
        "reasoningContent": "preserved reasoning"
    });
    let message: Message = serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(serde_json::to_value(&message).unwrap(), expected);
    assert_eq!(serde_json::to_value(message.clone()).unwrap(), expected);
    let restored: Message = serde_json::from_value(expected).unwrap();
    assert_ne!(message.revision(), restored.revision());
    assert_eq!(
        message, restored,
        "revision identity must not affect semantic equality"
    );
}
