//! DeepSeek's standalone Search/Fetch API. Both use the configured search
//! account; neither borrows an unrelated model provider's credentials.
use crate::app_settings::WebSearchCustomProviderConfig;
use crate::web_search::{SearchEngine, SearchProviderFailure, SearchRequest, SearchResultItem};
use futures::StreamExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

fn failure(code: &str, message: impl Into<String>) -> SearchProviderFailure {
    SearchProviderFailure::new(SearchEngine::Deepseek, code, message)
}

fn search_body(query: &str, limit: usize) -> Result<Value, SearchProviderFailure> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > 128 {
        return Err(failure("invalid_query", "DeepSeek Search requires a query of 1–128 characters. Shorten the query without changing its intent."));
    }
    Ok(json!({"query":query,"count":limit.clamp(1, 50),"need_content":false}))
}

fn operation_endpoint(base: &str, operation: &str) -> Result<reqwest::Url, SearchProviderFailure> {
    let mut url = reqwest::Url::parse(base)
        .map_err(|_| failure("invalid_base_url", "Invalid DeepSeek endpoint"))?;
    let path = url.path().trim_end_matches('/');
    let path = path
        .strip_suffix("/search")
        .or_else(|| path.strip_suffix("/fetch"))
        .unwrap_or(path);
    let root = if path.ends_with("/v1/tools") {
        path.to_string()
    } else if path.ends_with("/v1") {
        format!("{path}/tools")
    } else {
        format!("{path}/v1/tools")
    };
    url.set_path(&format!("{root}/{operation}"));
    Ok(url)
}

async fn post(
    provider: &WebSearchCustomProviderConfig,
    operation: &str,
    body: Value,
) -> Result<Value, SearchProviderFailure> {
    let endpoint = provider
        .effective_base_url()
        .ok_or_else(|| failure("not_configured", "Configure DeepSeek Search in settings"))?;
    let endpoint = operation_endpoint(&endpoint, operation)?;
    let url = crate::tools::fetch_url_tool::validate_url_for_fetch(endpoint.as_str())
        .await
        .map_err(|_| {
            failure(
                "invalid_base_url",
                "DeepSeek endpoint must be a public HTTP(S) URL",
            )
        })?;
    post_validated(provider, operation, url, body).await
}

async fn post_validated(
    provider: &WebSearchCustomProviderConfig,
    operation: &str,
    url: reqwest::Url,
    body: Value,
) -> Result<Value, SearchProviderFailure> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| failure("request_failed", "Unable to initialize DeepSeek web client"))?;
    post_with_client(&client, provider, operation, url, body).await
}

async fn post_with_client(
    client: &reqwest::Client,
    provider: &WebSearchCustomProviderConfig,
    operation: &str,
    url: reqwest::Url,
    body: Value,
) -> Result<Value, SearchProviderFailure> {
    'attempts: for attempt in 0..3 {
        // Search and Fetch have independent 5 QPS account limits. Reserve a
        // slot before transport; credentials never enter diagnostics or keys.
        static SLOTS: OnceLock<tokio::sync::Mutex<HashMap<String, tokio::time::Instant>>> =
            OnceLock::new();
        let slot = {
            let mut slots = SLOTS.get_or_init(Default::default).lock().await;
            let now = tokio::time::Instant::now();
            slots.retain(|_, time| *time + Duration::from_secs(60) > now);
            let key = format!(
                "{}:{operation}:{}",
                url.origin().ascii_serialization(),
                blake3::hash(provider.api_key.trim().as_bytes())
            );
            let slot = slots.get(&key).copied().unwrap_or(now).max(now);
            slots.insert(key, slot + Duration::from_millis(200));
            slot
        };
        tokio::time::sleep_until(slot).await;
        let response = match client
            .post(url.clone())
            .bearer_auth(provider.api_key.trim())
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                if attempt < 2 && crate::llm::transport::is_retriable_http_error(&error) {
                    tokio::time::sleep(Duration::from_secs(1 + attempt)).await;
                    continue;
                }
                return Err(failure(
                    "request_failed",
                    "DeepSeek web request could not be completed",
                ));
            }
        };
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        if matches!(status, 408 | 429 | 500..=599) && attempt < 2 && retry_after.unwrap_or(1) <= 5 {
            tokio::time::sleep(Duration::from_secs(retry_after.unwrap_or(1 + attempt))).await;
            continue;
        }
        if !response.status().is_success() {
            let code = match status {
                401 | 403 => "auth_failed",
                402 => "insufficient_balance",
                429 => "rate_limited",
                _ => "http_status",
            };
            return Err(
                failure(code, format!("DeepSeek {operation} returned HTTP {status}"))
                    .with_retry_after(retry_after),
            );
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    if attempt < 2 && crate::llm::transport::is_retriable_http_error(&error) {
                        tokio::time::sleep(Duration::from_secs(1 + attempt)).await;
                        continue 'attempts;
                    }
                    return Err(failure(
                        "body_read_failed",
                        "DeepSeek response could not be read",
                    ));
                }
            };
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err(failure(
                    "response_too_large",
                    "DeepSeek response exceeded 2 MiB",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        return serde_json::from_slice(&bytes)
            .map_err(|_| failure("invalid_response", "DeepSeek returned invalid JSON"));
    }
    unreachable!("bounded retries return on their final attempt")
}

fn parse_results(value: &Value) -> Result<Vec<SearchResultItem>, SearchProviderFailure> {
    let rows = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            failure(
                "invalid_response",
                "DeepSeek Search response has no results array",
            )
        })?;
    Ok(rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let url = row.get("url")?.as_str()?;
            let public = super::public_url_info(url)?;
            let text = |key: &str| row.get(key).and_then(Value::as_str).unwrap_or_default();
            let snippet = ["snippet", "summary", "content"]
                .iter()
                .map(|key| text(key))
                .find(|text| !text.is_empty())
                .unwrap_or_default();
            Some(SearchResultItem {
                rank: index + 1,
                provider_rank: index + 1,
                title: text("title").to_string(),
                url: public.url,
                display_url: public.display_url,
                source: public.source,
                snippet: snippet.chars().take(4000).collect(),
                engine: SearchEngine::Deepseek,
                resolved: true,
                confidence: "medium".into(),
            })
        })
        .collect())
}

pub(crate) async fn search(
    provider: &WebSearchCustomProviderConfig,
    request: &SearchRequest,
) -> Result<Vec<SearchResultItem>, SearchProviderFailure> {
    parse_results(
        &post(
            provider,
            "search",
            search_body(&request.effective_query, request.limit)?,
        )
        .await?,
    )
}

pub(crate) async fn fetch(
    provider: &WebSearchCustomProviderConfig,
    url: &str,
) -> Result<(String, String), SearchProviderFailure> {
    if url.chars().count() > 8192 {
        return Err(failure(
            "invalid_url",
            "DeepSeek Fetch URL exceeds 8192 characters",
        ));
    }
    crate::tools::fetch_url_tool::validate_url_for_fetch(url)
        .await
        .map_err(|_| {
            failure(
                "invalid_url",
                "Fetch requires a public HTTP(S) URL without credentials",
            )
        })?;
    let value = post(provider, "fetch", json!({"url":url})).await?;
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| {
            failure(
                "invalid_response",
                "DeepSeek Fetch returned no readable content",
            )
        })?;
    Ok((
        value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        content.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn transport_failures_retry_and_stop_after_three_attempts() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for fault in ["closed_headers", "timeout", "incomplete_body"] {
            for recover in [true, false] {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let url = reqwest::Url::parse(&format!(
                    "http://{}/v1/tools/fetch",
                    listener.local_addr().unwrap()
                ))
                .unwrap();
                let expected_attempts = if recover { 2 } else { 3 };
                let requests = Arc::new(AtomicUsize::new(0));
                let server_requests = requests.clone();
                let server = tokio::spawn(async move {
                    loop {
                        let (mut socket, _) = listener.accept().await.unwrap();
                        let attempt = server_requests.fetch_add(1, Ordering::SeqCst);
                        let mut headers = Vec::new();
                        while !headers.ends_with(b"\r\n\r\n") {
                            headers.push(socket.read_u8().await.unwrap());
                        }
                        let headers = String::from_utf8(headers).unwrap().to_ascii_lowercase();
                        assert!(headers.contains("authorization: bearer transport-test-key"));
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        let mut body = vec![0; length];
                        socket.read_exact(&mut body).await.unwrap();
                        assert_eq!(
                            serde_json::from_slice::<Value>(&body).unwrap(),
                            json!({"url":"https://example.com/"})
                        );
                        if recover && attempt == 1 {
                            let body = "{\"content\":\"recovered\"}";
                            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                        } else if fault == "timeout" {
                            tokio::time::sleep(Duration::from_millis(1100)).await;
                        } else if fault == "incomplete_body" {
                            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 99\r\nConnection: close\r\n\r\n{\"content\":").await.unwrap();
                        }
                    }
                });
                let provider = WebSearchCustomProviderConfig {
                    id: "transport-test".into(),
                    preset: crate::app_settings::WebSearchCustomProviderPreset::Deepseek,
                    name: "Test".into(),
                    enabled: true,
                    api_key: "transport-test-key".into(),
                    base_url: None,
                    priority: 0,
                };
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .timeout(Duration::from_secs(1))
                    .build()
                    .unwrap();
                let response = tokio::time::timeout(
                    Duration::from_secs(12),
                    post_with_client(
                        &client,
                        &provider,
                        "fetch",
                        url,
                        json!({"url":"https://example.com/"}),
                    ),
                )
                .await
                .unwrap();
                server.abort();
                let server_result = server.await.unwrap_err();
                assert!(
                    server_result.is_cancelled(),
                    "mock server failed: {server_result}"
                );
                assert_eq!(
                    requests.load(Ordering::SeqCst),
                    expected_attempts,
                    "fault: {fault}"
                );
                if recover {
                    assert_eq!(
                        response.unwrap(),
                        json!({"content":"recovered"}),
                        "fault: {fault}"
                    );
                } else {
                    assert_eq!(
                        response.unwrap_err().code,
                        if fault == "incomplete_body" {
                            "body_read_failed"
                        } else {
                            "request_failed"
                        }
                    );
                }
            }
        }
    }
    #[tokio::test]
    async fn wire_requests_preserve_auth_and_body_retry_only_transient_statuses() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for statuses in [vec![429, 200], vec![402]] {
            let expected_error = statuses[0] == 402;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = operation_endpoint(
                &format!("http://{}/v1", listener.local_addr().unwrap()),
                "search",
            )
            .unwrap();
            let server = tokio::spawn(async move {
                for status in statuses {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut headers = Vec::new();
                    while !headers.ends_with(b"\r\n\r\n") {
                        headers.push(socket.read_u8().await.unwrap());
                    }
                    let headers = String::from_utf8(headers).unwrap().to_ascii_lowercase();
                    assert!(headers.starts_with("post /v1/tools/search "));
                    assert!(headers.contains("authorization: bearer test-search-key"));
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    let mut body = vec![0; length];
                    socket.read_exact(&mut body).await.unwrap();
                    assert_eq!(
                        serde_json::from_slice::<Value>(&body).unwrap(),
                        json!({"query":"查证","count":3,"need_content":false})
                    );
                    let body = "{\"results\":[]}";
                    socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nRetry-After: 0\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                }
            });
            let provider = WebSearchCustomProviderConfig {
                id: "test".into(),
                preset: crate::app_settings::WebSearchCustomProviderPreset::Deepseek,
                name: "Test".into(),
                enabled: true,
                api_key: "test-search-key".into(),
                base_url: None,
                priority: 0,
            };
            let response = tokio::time::timeout(
                Duration::from_secs(5),
                post_validated(&provider, "search", url, search_body("查证", 3).unwrap()),
            )
            .await
            .unwrap();
            if expected_error {
                assert_eq!(response.unwrap_err().code, "insufficient_balance");
            } else {
                assert_eq!(response.unwrap(), json!({"results":[]}));
            }
            server.await.unwrap();
        }
    }
    #[test]
    fn endpoints_accept_roots_versions_and_complete_operations() {
        for base in [
            "https://api.deepseek.com",
            "https://api.deepseek.com/",
            "https://api.deepseek.com/v1",
            "https://api.deepseek.com/v1/tools/",
            "https://api.deepseek.com/v1/tools/search",
            "https://api.deepseek.com/v1/tools/fetch",
        ] {
            for operation in ["search", "fetch"] {
                assert_eq!(
                    operation_endpoint(base, operation).unwrap().as_str(),
                    format!("https://api.deepseek.com/v1/tools/{operation}")
                );
            }
        }
        assert_eq!(
            operation_endpoint("https://proxy.example/deepseek/v1", "fetch")
                .unwrap()
                .as_str(),
            "https://proxy.example/deepseek/v1/tools/fetch"
        );
    }

    #[test]
    fn search_contract_counts_unicode_and_preserves_intent() {
        assert_eq!(search_body(&"字".repeat(128), 100).unwrap()["count"], 50);
        assert!(search_body(&"字".repeat(129), 10).is_err());
        assert!(search_body("  ", 10).is_err());
        assert_eq!(
            search_body("DeepSeek Flash 1017", 8).unwrap(),
            json!({"query":"DeepSeek Flash 1017","count":8,"need_content":false})
        );
    }
    #[test]
    fn results_keep_sources_and_reject_malformed_responses() {
        let results = parse_results(&json!({"results":[{"title":"Official","url":"https://api-docs.deepseek.com/updates/","snippet":"release","rerank_score":0.9},{"url":"file:///private"}]})).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].engine, SearchEngine::Deepseek);
        assert_eq!(results[0].snippet, "release");
        assert!(parse_results(&json!({"error":"denied"})).is_err());
    }
}
