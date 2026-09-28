//! Compile conversation history into Responses input without rewriting native output.

use super::{provider_hosted_tool_identity, role_str};
use crate::error::CoreError;
use crate::llm::{ContentPart, Message, Role};

fn replayable_responses_reasoning(message: &Message) -> Vec<serde_json::Value> {
    if let Some(envelope) = message.provider_turn() {
        match &envelope.replay_payload {
            crate::llm::provider_turn::ProviderReplayPayload::DeepSeekResponseItems(payload)
            | crate::llm::provider_turn::ProviderReplayPayload::OpenAiResponseItems(payload) => {
                return payload.items.clone();
            }
            _ => {}
        }
    }
    message
        .tool_calls
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|tool_call| tool_call.thought_signature.as_deref())
        .find_map(crate::llm::provider_turn::decode_responses_reasoning_items)
        .map(|payload| payload.items)
        .unwrap_or_default()
}

fn responses_call_id(item: &serde_json::Value) -> Option<&str> {
    item.get("call_id")
        .and_then(serde_json::Value::as_str)
        .filter(|call_id| !call_id.trim().is_empty())
}

fn validate_responses_input_items(
    items: &[serde_json::Value],
    assistant_output_groups: &[std::ops::Range<usize>],
) -> Result<(), CoreError> {
    let mut call_ids = std::collections::HashSet::new();
    let mut output_ids = std::collections::HashSet::new();
    let mut pending_call_ids = std::collections::HashMap::new();

    for (index, item) in items.iter().enumerate() {
        let item_type = item
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("missing");
        // A provider response is an ordered group: reasoning, assistant text,
        // hosted tools and client calls can interleave inside it. Only a new
        // message/response must wait for every client output from that group.
        let continues_native_output = assistant_output_groups.iter().any(|group| {
            group.contains(&index)
                && pending_call_ids
                    .values()
                    .all(|call_index| group.contains(call_index))
        });
        match item_type {
            "function_call" => {
                let call_id = responses_call_id(item).ok_or_else(|| {
                    CoreError::Llm(format!(
                        "Responses input function_call at index {index} omitted call_id"
                    ))
                })?;
                if !call_ids.insert(call_id.to_string()) {
                    return Err(CoreError::Llm(format!(
                        "Responses input contains duplicate function_call call_id {call_id}"
                    )));
                }
                if !pending_call_ids.is_empty()
                    && assistant_output_groups
                        .iter()
                        .any(|group| group.start == index)
                {
                    return Err(CoreError::Llm(
                        "Responses input starts another response before pending tool outputs"
                            .into(),
                    ));
                }
                pending_call_ids.insert(call_id.to_string(), index);
            }
            "function_call_output" => {
                let call_id = responses_call_id(item).ok_or_else(|| {
                    CoreError::Llm(format!(
                        "Responses input function_call_output at index {index} omitted call_id"
                    ))
                })?;
                if !call_ids.contains(call_id) {
                    return Err(CoreError::Llm(format!(
                        "Responses input contains orphan function_call_output for call_id {call_id}"
                    )));
                }
                if !output_ids.insert(call_id.to_string()) {
                    return Err(CoreError::Llm(format!(
                        "Responses input contains duplicate function_call_output for call_id {call_id}"
                    )));
                }
                pending_call_ids.remove(call_id);
            }
            "message" | "reasoning" if !pending_call_ids.is_empty() && !continues_native_output => {
                let mut pending = pending_call_ids.keys().cloned().collect::<Vec<_>>();
                pending.sort();
                return Err(CoreError::Llm(format!(
                    "Responses input places {item_type} before output for pending call_id(s): {}",
                    pending.join(", ")
                )));
            }
            item_type
                if provider_hosted_tool_identity(item_type, item).is_some()
                    && !pending_call_ids.is_empty()
                    && !continues_native_output =>
            {
                let mut pending = pending_call_ids.keys().cloned().collect::<Vec<_>>();
                pending.sort();
                return Err(CoreError::Llm(format!(
                    "Responses input places {item_type} before output for pending call_id(s): {}",
                    pending.join(", ")
                )));
            }
            _ => {}
        }
    }

    if pending_call_ids.is_empty() {
        return Ok(());
    }
    let mut pending = pending_call_ids.into_keys().collect::<Vec<_>>();
    pending.sort();
    Err(CoreError::Llm(format!(
        "Responses input ended without function_call_output for call_id(s): {}",
        pending.join(", ")
    )))
}

pub(super) fn responses_input_items(
    messages: &[Message],
) -> Result<Vec<serde_json::Value>, CoreError> {
    let mut items = Vec::new();
    let mut assistant_output_groups = Vec::new();
    let leading_system_count = messages
        .iter()
        .take_while(|message| message.role == Role::System)
        .count();
    for (message_index, message) in messages.iter().enumerate() {
        if message.role == Role::Tool {
            items.push(serde_json::json!({
                "type": "function_call_output",
                "call_id": message.name,
                "output": message.text_content(),
            }));
            continue;
        }

        let mut replayed_call_ids = std::collections::HashSet::new();
        let mut replayed_message = false;
        let mut replay_items = Vec::new();
        if message.role == Role::Assistant {
            replay_items = replayable_responses_reasoning(message);
            replayed_message = replay_items.iter().any(|item| {
                item.get("type").and_then(serde_json::Value::as_str) == Some("message")
            });
            replayed_call_ids.extend(replay_items.iter().filter_map(|item| {
                (item.get("type").and_then(serde_json::Value::as_str) == Some("function_call"))
                    .then(|| {
                        item.get("call_id")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                    })
                    .flatten()
            }));
        }
        let assistant_output_start = (message.role == Role::Assistant).then_some(items.len());

        let mut content = Vec::new();
        for part in &message.parts {
            match part {
                ContentPart::Text { text } => content.push(serde_json::json!({
                    "type": if message.role == Role::Assistant { "output_text" } else { "input_text" },
                    "text": text,
                })),
                ContentPart::Image { media_type, data } => content.push(serde_json::json!({
                    "type": "input_image",
                    "image_url": format!("data:{media_type};base64,{data}"),
                })),
                ContentPart::ProviderTurn { .. } => {}
            }
        }
        let generic_message = (!(content.is_empty()
            || message.role == Role::Assistant && replayed_message))
            .then(|| {
                let wire_role =
                    if message.role == Role::System && message_index >= leading_system_count {
                        // OpenAI Responses supports developer messages; DeepSeek
                        // Responses explicitly treats them as user/controller
                        // input. This preserves append-only prompt order without
                        // presenting runtime state as human-authored text.
                        "developer"
                    } else {
                        role_str(&message.role)
                    };
                serde_json::json!({
                "type": "message",
                "role": wire_role,
                "content": content,
                })
            });
        if message.role == Role::Assistant {
            let function_state = replay_items.split_off(
                replay_items
                    .iter()
                    .position(|item| {
                        item.get("type").and_then(serde_json::Value::as_str)
                            == Some("function_call")
                    })
                    .unwrap_or(replay_items.len()),
            );
            items.extend(replay_items);
            items.extend(generic_message);
            items.extend(function_state);
        } else {
            items.extend(generic_message);
        }
        for tool_call in message.tool_calls.as_deref().unwrap_or_default() {
            if replayed_call_ids.contains(tool_call.id.as_str()) {
                continue;
            }
            items.push(serde_json::json!({
                "type": "function_call",
                "call_id": tool_call.id,
                "name": tool_call.name,
                "arguments": tool_call.arguments,
            }));
        }
        if let Some(start) = assistant_output_start {
            assistant_output_groups.push(start..items.len());
        }
    }
    validate_responses_input_items(&items, &assistant_output_groups)?;
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::openai::parse_responses_completion;

    #[test]
    fn responses_tool_loop_replays_encrypted_reasoning_before_function_state() {
        let capability = crate::model_catalog::NativeWebSearchCapability {
            dialect: crate::llm::native_search::NativeSearchDialect::OpenAiResponses,
            supports_domains: true,
            supports_recency: false,
            supports_locale: false,
            supports_location: true,
            supports_citations: true,
            supports_stream_events: true,
            can_mix_client_tools: true,
        };
        let response = parse_responses_completion(
            serde_json::json!({
                "status": "completed",
                "output": [
                    {
                        "type": "reasoning",
                        "id": "rs_1",
                        "status": "completed",
                        "encrypted_content": "encrypted-reasoning",
                        "summary": []
                    },
                    {
                        "type": "function_call",
                        "id": "fc_1",
                        "status": "completed",
                        "call_id": "call_1",
                        "name": "read_file",
                        "arguments": "{\"path\":\"README.md\"}"
                    }
                ]
            }),
            crate::llm::native_search::NativeSearchDialect::OpenAiResponses,
            capability,
        )
        .unwrap();
        let mut assistant = Message::text(Role::Assistant, "");
        assistant.parts.clear();
        assistant.tool_calls = response.tool_calls;
        let tool_result = Message::text_with_name(Role::Tool, "contents", "call_1");

        let replay = responses_input_items(&[assistant, tool_result]).unwrap();
        assert_eq!(replay[0]["type"], "reasoning");
        assert_eq!(replay[0]["encrypted_content"], "encrypted-reasoning");
        assert_eq!(replay[1]["type"], "function_call");
        assert_eq!(replay[2]["type"], "function_call_output");
    }

    #[test]
    fn responses_replay_inserts_generic_message_before_unresolved_function_state() {
        let capability = crate::model_catalog::NativeWebSearchCapability {
            dialect: crate::llm::native_search::NativeSearchDialect::OpenAiResponses,
            supports_domains: true,
            supports_recency: false,
            supports_locale: false,
            supports_location: true,
            supports_citations: true,
            supports_stream_events: true,
            can_mix_client_tools: true,
        };
        let response = parse_responses_completion(
            serde_json::json!({
                "status": "completed",
                "output": [
                    {
                        "type": "reasoning",
                        "id": "rs_1",
                        "status": "completed",
                        "encrypted_content": "opaque",
                        "summary": []
                    },
                    {
                        "type": "function_call",
                        "id": "fc_1",
                        "status": "completed",
                        "call_id": "call_1",
                        "name": "read_file",
                        "arguments": "{\"path\":\"README.md\"}"
                    }
                ]
            }),
            crate::llm::native_search::NativeSearchDialect::OpenAiResponses,
            capability,
        )
        .unwrap();
        let mut assistant = Message::text(Role::Assistant, "Working on it.");
        assistant.tool_calls = response.tool_calls;
        let tool_result = Message::text_with_name(Role::Tool, "contents", "call_1");

        let replay = responses_input_items(&[assistant, tool_result]).unwrap();
        assert_eq!(
            replay
                .iter()
                .map(|item| item["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "reasoning",
                "message",
                "function_call",
                "function_call_output"
            ]
        );
    }

    #[test]
    fn responses_replay_preserves_interleaved_parallel_native_output() {
        let capability = crate::model_catalog::NativeWebSearchCapability {
            dialect: crate::llm::native_search::NativeSearchDialect::OpenAiResponses,
            supports_domains: true,
            supports_recency: false,
            supports_locale: false,
            supports_location: true,
            supports_citations: true,
            supports_stream_events: true,
            can_mix_client_tools: true,
        };
        for dialect in [
            crate::llm::native_search::NativeSearchDialect::OpenAiResponses,
            crate::llm::native_search::NativeSearchDialect::DeepSeekResponses,
        ] {
            let mut output = Vec::new();
            for index in 0..4 {
                let mut reasoning = serde_json::json!({
                    "type": "reasoning", "id": format!("rs_{index}"),
                    "status": "completed", "encrypted_content": format!("opaque_{index}"),
                    "summary": [],
                });
                if dialect == crate::llm::native_search::NativeSearchDialect::DeepSeekResponses {
                    reasoning
                        .as_object_mut()
                        .unwrap()
                        .remove("encrypted_content");
                    reasoning["content"] = serde_json::json!([{"type":"reasoning_text", "text":format!("reasoning {index}")}]);
                }
                output.push(reasoning);
                output.push(serde_json::json!({
                    "type": "function_call", "id": format!("fc_{index}"),
                    "status": "completed", "call_id": format!("call_{index}"),
                    "name": "spawn_subagent", "arguments": "{\"task\":\"inspect\"}",
                }));
            }
            let response = parse_responses_completion(
                serde_json::json!({"status": "completed", "output": output}),
                dialect,
                capability.clone(),
            )
            .unwrap();
            let mut assistant = Message::text(Role::Assistant, "");
            assistant.parts.clear();
            assistant.tool_calls = response.tool_calls;
            let mut messages = vec![assistant];
            // Parallel workers may finish in a different order from their calls.
            for index in (0..4).rev() {
                messages.push(Message::text_with_name(
                    Role::Tool,
                    "worker started",
                    format!("call_{index}"),
                ));
            }
            let replay = responses_input_items(&messages)
                .expect("one completed native output may interleave reasoning and parallel calls");
            assert_eq!(&replay[..output.len()], output.as_slice());
            assert_eq!(replay.len(), output.len() + 4);

            let mut missing_output = messages.clone();
            missing_output.pop();
            assert!(responses_input_items(&missing_output)
                .unwrap_err()
                .to_string()
                .contains("ended without function_call_output"));
            let mut crossed_response = messages.clone();
            crossed_response.insert(1, Message::text(Role::Assistant, "a later response"));
            assert!(responses_input_items(&crossed_response)
                .unwrap_err()
                .to_string()
                .contains("message before output"));
            let mut crossed_calls = messages.clone();
            let mut later_call = Message::text(Role::Assistant, "");
            later_call.parts.clear();
            later_call.tool_calls = Some(vec![crate::llm::ToolCallRequest {
                id: "later_call".into(),
                name: "read_file".into(),
                arguments: "{}".into(),
                thought_signature: None,
            }]);
            crossed_calls.insert(1, later_call);
            assert!(responses_input_items(&crossed_calls)
                .unwrap_err()
                .to_string()
                .contains("another response before pending tool outputs"));
        }
    }

    #[test]
    fn responses_wire_validation_accepts_parallel_call_batch() {
        let items = vec![
            serde_json::json!({"type": "function_call", "call_id": "call_1", "name": "read_file", "arguments": "{}"}),
            serde_json::json!({"type": "function_call", "call_id": "call_2", "name": "read_file", "arguments": "{}"}),
            serde_json::json!({"type": "function_call_output", "call_id": "call_1", "output": "a"}),
            serde_json::json!({"type": "function_call_output", "call_id": "call_2", "output": "b"}),
        ];

        validate_responses_input_items(&items, &[]).expect("parallel call batches are valid");
    }

    #[test]
    fn responses_wire_validation_rejects_broken_call_output_sequences() {
        let cases = [
            (
                vec![serde_json::json!({"type": "function_call", "name": "read_file"})],
                "omitted call_id",
            ),
            (
                vec![
                    serde_json::json!({"type": "function_call_output", "call_id": "orphan", "output": "x"}),
                ],
                "orphan function_call_output",
            ),
            (
                vec![
                    serde_json::json!({"type": "function_call", "call_id": "call_1", "name": "read_file", "arguments": "{}"}),
                    serde_json::json!({"type": "function_call", "call_id": "call_1", "name": "read_file", "arguments": "{}"}),
                ],
                "duplicate function_call call_id call_1",
            ),
            (
                vec![
                    serde_json::json!({"type": "function_call", "call_id": "call_1", "name": "read_file", "arguments": "{}"}),
                    serde_json::json!({"type": "function_call_output", "call_id": "call_1", "output": "x"}),
                    serde_json::json!({"type": "function_call_output", "call_id": "call_1", "output": "x"}),
                ],
                "duplicate function_call_output",
            ),
            (
                vec![
                    serde_json::json!({"type": "function_call", "call_id": "call_1", "name": "read_file", "arguments": "{}"}),
                    serde_json::json!({"type": "message", "role": "assistant", "content": []}),
                    serde_json::json!({"type": "function_call_output", "call_id": "call_1", "output": "x"}),
                ],
                "message before output for pending call_id(s): call_1",
            ),
            (
                vec![
                    serde_json::json!({"type": "function_call", "call_id": "call_1", "name": "read_file", "arguments": "{}"}),
                ],
                "ended without function_call_output for call_id(s): call_1",
            ),
        ];

        for (items, expected) in cases {
            let error = validate_responses_input_items(&items, &[])
                .expect_err("broken Responses history must fail before transport");
            assert!(error.to_string().contains(expected), "{error}");
        }
    }
}
