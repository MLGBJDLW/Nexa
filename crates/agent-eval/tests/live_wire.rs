use std::sync::{Arc, Mutex};
use std::time::Duration;

use nexa_agent_eval::{run_suite, EvalConfig, EvalMode};
use nexa_core::llm::ProviderType;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Local HTTP peer proves that --live uses the real wire adapter and executor.
/// It is not an external-model quality measurement and writes no baseline file.
#[tokio::test]
async fn live_mode_crosses_provider_wire_and_real_file_execution() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = requests.clone();
    let server = tokio::spawn(async move {
        for index in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let (header_end, content_length) = loop {
                let mut chunk = [0_u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
                if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    break (end + 4, length);
                }
            };
            while bytes.len() < header_end + content_length {
                let mut chunk = [0_u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
            }
            captured.lock().unwrap().push(
                serde_json::from_slice(&bytes[header_end..header_end + content_length]).unwrap(),
            );
            let delta = match index {
                0 => {
                    json!({"tool_calls":[{"index":0,"id":"read","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"src/module.mjs\"}"}}]})
                }
                1 => {
                    json!({"tool_calls":[{"index":0,"id":"edit","type":"function","function":{"name":"edit_file","arguments":json!({"path":"src/module.mjs","old_str":"return a-b;","new_str":"return a+b;"}).to_string()}}]})
                }
                _ => json!({"content":"Implemented the requested addition fix."}),
            };
            let chunk = json!({"id":format!("response-{index}"),"choices":[{"index":0,"delta":delta,"finish_reason":if index < 2 { "tool_calls" } else { "stop" }}],"usage":{"prompt_tokens":50,"completion_tokens":10,"total_tokens":60}});
            let body = format!("data: {chunk}\n\ndata: [DONE]\n\n");
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
    });
    let config = EvalConfig {
        model: "wire-fixture".into(),
        provider: ProviderType::OpenAi,
        base_url: Some(format!("http://{address}/v1")),
        timeout_seconds: 15,
        reasoning_enabled: Some(false),
        ..Default::default()
    };
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        run_suite(
            EvalMode::Live,
            &config,
            Some("fixture-key"),
            Some("addition-keeps-api"),
        ),
    )
    .await;
    server.abort();
    let report = result.unwrap().unwrap();
    assert_eq!((report.passed, report.total), (1, 1), "{:#?}", report.cases);
    assert_eq!(report.cases[0].tool_calls, 2);
    assert_eq!(report.cases[0].provider_invocations.len(), 3);
    assert_eq!(
        report.cases[0]
            .provider_invocations
            .iter()
            .map(|call| call.prompt_tokens)
            .sum::<u32>(),
        150
    );
    assert_eq!(
        report.cases[0].reported_usage_cost, None,
        "missing pricing is not zero cost"
    );
    assert!(requests.lock().unwrap()[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["role"] == "tool"
            && message["content"]
                .as_str()
                .is_some_and(|text| text.contains("return a-b"))));
}
