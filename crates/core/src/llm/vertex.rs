//! Vertex routes share Google's and Anthropic's native codecs. Credentials
//! remain endpoint scoped: Express API key or project-scoped OAuth access token.
use std::{collections::HashSet, time::Duration};

use serde_json::Value;

use super::{with_request_timeout, ProviderConfig, ProviderType};
use crate::error::CoreError;

pub const DEFAULT_BASE_URL: &str = "https://aiplatform.googleapis.com/v1/publishers/google";

pub fn is_anthropic(config: &ProviderConfig) -> bool {
    config.provider_type == ProviderType::Vertex
        && config.base_url.as_deref().is_some_and(|base| {
            base.trim_end_matches('/')
                .ends_with("/publishers/anthropic")
        })
}

pub fn is_project_route(base_url: &str) -> bool {
    reqwest::Url::parse(base_url).is_ok_and(|url| url.path().contains("/projects/"))
}

pub fn authorize(
    request: reqwest::RequestBuilder,
    base_url: &str,
    credential: &str,
) -> reqwest::RequestBuilder {
    if is_project_route(base_url) {
        request.bearer_auth(credential)
    } else {
        request.header("x-goog-api-key", credential)
    }
}

pub fn inference_url(base_url: &str, model: &str, streaming: bool) -> Result<String, CoreError> {
    let mut url = reqwest::Url::parse(base_url)
        .map_err(|_| CoreError::InvalidInput("Invalid Vertex base URL".into()))?;
    let action = if streaming {
        "streamRawPredict"
    } else {
        "rawPredict"
    };
    url.path_segments_mut()
        .map_err(|_| CoreError::InvalidInput("Invalid Vertex base URL".into()))?
        .pop_if_empty()
        .push("models")
        .push(&format!("{model}:{action}"));
    Ok(url.into())
}

pub fn anthropic_body<T: serde::Serialize>(body: &T) -> Result<Value, CoreError> {
    let mut body = serde_json::to_value(body)?;
    let model = body["model"].as_str().unwrap_or_default();
    let beta = super::anthropic::cloud_beta_flags(model);
    body.as_object_mut()
        .ok_or_else(|| CoreError::Llm("Invalid Claude request body".into()))?
        .remove("model");
    if !beta.is_empty() {
        body["anthropic_beta"] = serde_json::json!(beta);
    }
    body["anthropic_version"] = "vertex-2023-10-16".into();
    Ok(body)
}

/// Publisher discovery is independent from model invocation permission. The UI
/// may still offer manual model/profile entry when discovery is unavailable.
pub async fn list_models(
    client: &reqwest::Client,
    base_url: &str,
    credential: &str,
    timeout: Option<Duration>,
) -> Result<Vec<String>, CoreError> {
    let mut url = reqwest::Url::parse(base_url)
        .map_err(|_| CoreError::InvalidInput("Invalid Vertex base URL".into()))?;
    let publisher = if url.path().trim_end_matches('/').ends_with("/anthropic") {
        "anthropic"
    } else {
        "google"
    };
    url.set_path(&format!("/v1beta1/publishers/{publisher}/models"));
    let mut cursor = None::<String>;
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    loop {
        crate::privacy::runtime::ensure_invocation_current()?;
        let mut request =
            authorize(client.get(url.clone()), base_url, credential).query(&[("pageSize", "100")]);
        if let Some(cursor) = &cursor {
            request = request.query(&[("pageToken", cursor)]);
        }
        let response = with_request_timeout(request, timeout)
            .send()
            .await
            .map_err(|error| {
                super::transport::classify_http_error(&error, "Vertex model discovery")
            })?;
        let status = response.status();
        let body = response.text().await.map_err(|error| {
            super::transport::classify_http_error(&error, "Vertex model discovery")
        })?;
        if !status.is_success() {
            return Err(CoreError::Llm(format!("Vertex catalog HTTP {status}: {}. Catalog permissions and inference access are separate.", crate::sensitive_data::sanitize_diagnostic(&body, Some(credential)))));
        }
        let value: Value = serde_json::from_str(&body)?;
        let items = value["publisherModels"]
            .as_array()
            .ok_or_else(|| CoreError::Llm("Vertex catalog has no publisherModels array".into()))?;
        models.extend(
            items
                .iter()
                .filter_map(|item| item["name"].as_str())
                .filter_map(|name| name.rsplit('/').next())
                .map(str::to_owned),
        );
        cursor = value["nextPageToken"]
            .as_str()
            .filter(|token| !token.is_empty())
            .map(str::to_owned);
        let Some(next) = &cursor else { break };
        if !seen.insert(next.clone()) {
            return Err(CoreError::Llm(
                "Vertex catalog repeated a pagination token".into(),
            ));
        }
    }
    models.sort();
    models.dedup();
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_and_claude_payload_follow_the_selected_route() {
        let client = reqwest::Client::new();
        let express = authorize(client.get(DEFAULT_BASE_URL), DEFAULT_BASE_URL, "secret")
            .build()
            .unwrap();
        assert_eq!(express.headers()["x-goog-api-key"], "secret");
        assert!(!express.headers().contains_key("authorization"));
        let project = "https://us-central1-aiplatform.googleapis.com/v1/projects/demo/locations/us-central1/publishers/anthropic";
        let oauth = authorize(client.get(project), project, "token")
            .build()
            .unwrap();
        assert_eq!(oauth.headers()["authorization"], "Bearer token");
        assert!(!oauth.headers().contains_key("x-goog-api-key"));
        assert!(inference_url(project, "claude-opus-5-5", true)
            .unwrap()
            .ends_with("/models/claude-opus-5-5:streamRawPredict"));
        let body = anthropic_body(
            &serde_json::json!({"model":"claude-opus-5-5", "messages":[], "max_tokens":128}),
        )
        .unwrap();
        assert!(body.get("model").is_none());
        assert_eq!(body["anthropic_version"], "vertex-2023-10-16");
    }

    #[tokio::test]
    async fn native_google_and_claude_routes_stream_without_rewriting_protocols() {
        use crate::llm::{
            create_provider, CompletionRequest, FinishReason, Message, ProviderStreamEvent, Role,
        };
        use futures::StreamExt;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (publisher, project) in [("google", false), ("google", true), ("anthropic", true)] {
            for streaming in [false, true] {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let path = if project {
                    format!("/v1/projects/demo/locations/us-central1/publishers/{publisher}")
                } else {
                    format!("/v1/publishers/{publisher}")
                };
                let model = if publisher == "google" {
                    "gemini-3.5-flash"
                } else {
                    "claude-opus-5-5"
                };
                let expected_path = format!(
                    "{path}/models/{model}:{}",
                    match (publisher, streaming) {
                        ("google", true) => "streamGenerateContent?alt=sse",
                        ("google", false) => "generateContent",
                        (_, true) => "streamRawPredict",
                        (_, false) => "rawPredict",
                    }
                );
                let server = tokio::spawn(async move {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut input = Vec::new();
                    let body_start = loop {
                        let mut chunk = [0u8; 4096];
                        let read = socket.read(&mut chunk).await.unwrap();
                        assert!(read > 0);
                        input.extend_from_slice(&chunk[..read]);
                        if let Some(end) = input.windows(4).position(|part| part == b"\r\n\r\n") {
                            let headers = String::from_utf8_lossy(&input[..end]);
                            let length = headers
                                .lines()
                                .find_map(|line| {
                                    line.to_ascii_lowercase()
                                        .strip_prefix("content-length:")
                                        .map(|length| length.trim().parse::<usize>().unwrap())
                                })
                                .unwrap();
                            if input.len() >= end + 4 + length {
                                break end + 4;
                            }
                        }
                    };
                    let headers = String::from_utf8_lossy(&input[..body_start]);
                    assert!(headers.contains(&format!("POST {expected_path} ")));
                    assert!(headers.contains(if project {
                        "authorization: Bearer test-token"
                    } else {
                        "x-goog-api-key: test-token"
                    }));
                    assert!(!headers.contains("x-api-key:"));
                    let body: Value = serde_json::from_slice(&input[body_start..]).unwrap();
                    if publisher == "anthropic" {
                        assert_eq!(body["anthropic_version"], "vertex-2023-10-16");
                        assert!(body.get("model").is_none());
                        assert!(body["messages"].is_array());
                    } else {
                        assert!(body["contents"].is_array());
                        assert!(body.get("messages").is_none());
                    }
                    let value = if publisher == "google" {
                        serde_json::json!({"candidates":[{"content":{"role":"model","parts":[{"text":"Vertex works"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":1,"totalTokenCount":4}})
                    } else {
                        serde_json::json!({"id":"msg-1","type":"message","role":"assistant","model":model,"content":[{"type":"text","text":"Vertex works"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":1}})
                    };
                    let output = if !streaming {
                        value.to_string()
                    } else if publisher == "google" {
                        format!("data: {value}\n\n")
                    } else {
                        [
                            ("message_start", serde_json::json!({"type":"message_start","message":{"id":"msg-1","type":"message","role":"assistant","model":model,"content":[],"usage":{"input_tokens":3,"output_tokens":0}}})),
                            ("content_block_start", serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}})),
                            ("content_block_delta", serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Vertex works"}})),
                            ("content_block_stop", serde_json::json!({"type":"content_block_stop","index":0})),
                            ("message_delta", serde_json::json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}})),
                            ("message_stop", serde_json::json!({"type":"message_stop"})),
                        ].into_iter().map(|(event, value)| format!("event: {event}\ndata: {value}\n\n")).collect()
                    };
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", if streaming { "text/event-stream" } else { "application/json" }, output.len()).as_bytes()).await.unwrap();
                    socket.write_all(output.as_bytes()).await.unwrap();
                });
                let provider = create_provider(ProviderConfig {
                    provider_type: ProviderType::Vertex,
                    base_url: Some(format!("http://{address}{path}")),
                    api_key: Some("test-token".into()),
                    org_id: None,
                    timeout_secs: Some(3),
                    streaming: Default::default(),
                })
                .unwrap();
                let request = CompletionRequest {
                    model: model.into(),
                    messages: vec![Message::text(Role::User, "hello")],
                    max_tokens: Some(128),
                    ..Default::default()
                };
                if streaming {
                    let events: Vec<_> = provider
                        .stream_events(&request)
                        .await
                        .unwrap()
                        .collect()
                        .await;
                    assert!(events.iter().any(|event| matches!(event, ProviderStreamEvent::Chunk {chunk} if chunk.delta == "Vertex works")), "{publisher}: {events:?}");
                    assert!(events.iter().any(|event| matches!(event, ProviderStreamEvent::Chunk {chunk} if chunk.finish_reason == Some(FinishReason::Stop))), "{publisher}: {events:?}");
                } else {
                    assert_eq!(
                        provider.complete(&request).await.unwrap().content,
                        "Vertex works"
                    );
                }
                server.await.unwrap();
            }
        }
    }
}
