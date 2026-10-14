//! ACP per-turn usage is optional and distinct from context occupancy/account quota.
use super::{error, transport, Result};
use nexa_core::external_agent::{preset, ExternalAgentLaunch};
use serde::Serialize;
use serde_json::Value;
use std::{process::Stdio, time::Duration};
use tokio::io::AsyncReadExt;

const MAX_USAGE_BYTES: u64 = 256 * 1024;

#[derive(Default)]
pub(super) struct SessionUsage {
    previous: Option<nexa_core::llm::Usage>,
    prompted: bool,
}

impl SessionUsage {
    pub(super) fn observe(
        &mut self,
        provider: &str,
        response: &Value,
    ) -> Option<nexa_core::llm::Usage> {
        let current = prompt_usage(response);
        let already_prompted = std::mem::replace(&mut self.prompted, true);
        if provider != "hermes" {
            return current;
        }
        // Hermes exports session counters in PromptResponse, unlike Gemini
        // CLI's per-prompt usage. This baseline belongs to the pooled session.
        let previous = std::mem::replace(&mut self.previous, current.clone());
        let current = current?;
        let Some(previous) = previous else {
            return (!already_prompted).then_some(current);
        };
        if current.total_tokens < previous.total_tokens
            || current.prompt_tokens < previous.prompt_tokens
            || current.completion_tokens < previous.completion_tokens
        {
            // The runtime rebuilt its agent/reset its native session counters.
            return Some(current);
        }
        Some(nexa_core::llm::Usage {
            prompt_tokens: current.prompt_tokens - previous.prompt_tokens,
            completion_tokens: current.completion_tokens - previous.completion_tokens,
            total_tokens: current.total_tokens - previous.total_tokens,
            thinking_tokens: current
                .thinking_tokens
                .map(|v| v.saturating_sub(previous.thinking_tokens.unwrap_or(0))),
            cache_read_tokens: current
                .cache_read_tokens
                .map(|v| v.saturating_sub(previous.cache_read_tokens.unwrap_or(0))),
            cache_creation_tokens: current
                .cache_creation_tokens
                .map(|v| v.saturating_sub(previous.cache_creation_tokens.unwrap_or(0))),
            ..Default::default()
        })
    }
}

/// End-turn usage is implemented by some ACP adapters before it is universally
/// available. Accept both JSON wire casing and existing adapter casing; absent
/// or malformed counters remain unknown. Never read `used`, `size`, or `cost`.
pub(super) fn prompt_usage(value: &Value) -> Option<nexa_core::llm::Usage> {
    let usage = value.get("usage")?;
    let tokens = |camel: &str, snake: &str| {
        usage
            .get(camel)
            .or_else(|| usage.get(snake))
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
    };
    let prompt = tokens("inputTokens", "input_tokens")?;
    let completion = tokens("outputTokens", "output_tokens")?;
    let total = tokens("totalTokens", "total_tokens")?;
    Some(nexa_core::llm::Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        thinking_tokens: tokens("thoughtTokens", "thought_tokens"),
        cache_read_tokens: tokens("cachedReadTokens", "cached_read_tokens"),
        cache_creation_tokens: tokens("cachedWriteTokens", "cached_write_tokens"),
        ..Default::default()
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountUsageWindow {
    label: String,
    used_percent: Option<f64>,
    resets_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountUsageSnapshot {
    pub status: &'static str,
    provider: Option<String>,
    plan: Option<String>,
    fetched_at: Option<String>,
    windows: Vec<AccountUsageWindow>,
    details: Vec<String>,
}

impl AccountUsageSnapshot {
    pub(crate) fn unsupported() -> Self {
        Self {
            status: "unsupported",
            provider: None,
            plan: None,
            fetched_at: None,
            windows: vec![],
            details: vec![],
        }
    }
}

fn text(value: &Value, key: &str, max: usize) -> Option<String> {
    value
        .get(key)?
        .as_str()
        .map(|s| s.chars().filter(|c| !c.is_control()).take(max).collect())
}

fn timestamp(value: &Value, key: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(value.get(key)?.as_str()?)
        .ok()
        .map(|date| date.to_rfc3339())
}

fn project(value: &Value, expected_provider: &str) -> Result<AccountUsageSnapshot> {
    if value.get("provider").and_then(Value::as_str) != Some(expected_provider) {
        return Err(error(
            "Account usage provider does not match the selected Hermes model",
        ));
    }
    let unavailable = value
        .get("unavailable_reason")
        .is_some_and(|reason| !reason.is_null());
    let windows: Vec<_> = if unavailable {
        vec![]
    } else {
        value
            .get("windows")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(32)
            .map(|window| AccountUsageWindow {
                label: text(window, "label", 100).unwrap_or_default(),
                used_percent: window
                    .get("used_percent")
                    .and_then(Value::as_f64)
                    .filter(|used| used.is_finite())
                    .map(|used| used.clamp(0.0, 100.0)),
                resets_at: timestamp(window, "resets_at"),
            })
            .collect()
    };
    let details: Vec<_> = if unavailable {
        vec![]
    } else {
        value
            .get("details")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(8)
            .filter_map(Value::as_str)
            .map(|s| s.chars().filter(|c| !c.is_control()).take(300).collect())
            .collect()
    };
    Ok(AccountUsageSnapshot {
        status: if windows.is_empty() && details.is_empty() {
            "unavailable"
        } else {
            "ready"
        },
        provider: Some(expected_provider.to_owned()),
        plan: text(value, "plan", 80),
        fetched_at: timestamp(value, "fetched_at"),
        windows,
        details,
    })
}

fn hermes_usage_args(launch: &ExternalAgentLaunch, model: &str) -> Option<(Vec<String>, String)> {
    // Rewriting arbitrary wrapper argv can change the account/profile. Only the
    // bundled Hermes ACP invocation is understood; env (including HERMES_HOME)
    // and an explicit native executable still come from the saved launch.
    if launch
        .args
        .as_ref()
        .is_some_and(|args| args.as_slice() != ["acp"])
        || !launch.config_options.is_empty()
    {
        return None;
    }
    // Hermes ACP advertises provider:model identities. Do not guess a provider
    // from a model brand, or fall back to a different configured account.
    let (provider, model) = model.split_once(':')?;
    if model.is_empty()
        || provider.is_empty()
        || provider == "custom"
        || !provider.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
    {
        return None;
    }
    Some((
        vec![
            "usage".into(),
            "--json".into(),
            "--provider".into(),
            provider.into(),
        ],
        provider.into(),
    ))
}

async fn read_command(mut command: tokio::process::Command, timeout: Duration) -> Result<Value> {
    // Quota probes consume only JSON stdout; no protocol Wire drains stderr.
    command.stdin(Stdio::null()).stderr(Stdio::null());
    let (mut child, _tree) = nexa_core::managed_process::spawn(&mut command)
        .map_err(|_| error("Account usage runtime is unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| error("Account usage output unavailable"))?;
    let result = tokio::time::timeout(timeout, async {
        let mut output = Vec::new();
        stdout
            .take(MAX_USAGE_BYTES + 1)
            .read_to_end(&mut output)
            .await
            .map_err(|_| error("Account usage output could not be read"))?;
        if output.len() as u64 > MAX_USAGE_BYTES {
            return Err(error("Account usage response is too large"));
        }
        if !child
            .wait()
            .await
            .map_err(|_| error("Account usage runtime failed"))?
            .success()
        {
            return Err(error(
                "Account usage unavailable; check the agent version and sign-in",
            ));
        }
        serde_json::from_slice(&output)
            .map_err(|_| error("Account usage response is not valid JSON"))
    })
    .await;
    // Managed process containment kills descendants on success, failure and timeout.
    result.map_err(|_| error("Account usage request timed out"))?
}

pub(crate) async fn read_account_usage(
    provider: &str,
    launch: &ExternalAgentLaunch,
    model: &str,
) -> Result<AccountUsageSnapshot> {
    if provider != "hermes" {
        return Ok(AccountUsageSnapshot::unsupported());
    }
    let Some((args, account_provider)) = hermes_usage_args(launch, model) else {
        return Ok(AccountUsageSnapshot::unsupported());
    };
    let preset = preset(provider).ok_or_else(|| error("Unknown external agent"))?;
    let command = transport::command(preset, launch, &args)?;
    project(
        &read_command(command, Duration::from_secs(25)).await?,
        &account_provider,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn warm_hermes_sessions_bill_only_new_usage_and_recognize_counter_resets() {
        let snapshot = |input, output| json!({"usage":{"inputTokens":input,"outputTokens":output,"totalTokens":input+output}});
        let mut session = SessionUsage::default();
        assert_eq!(
            session
                .observe("hermes", &snapshot(100, 20))
                .unwrap()
                .total_tokens,
            120
        );
        assert_eq!(
            session
                .observe("hermes", &snapshot(160, 30))
                .unwrap()
                .total_tokens,
            70
        );
        assert_eq!(
            session
                .observe("hermes", &snapshot(160, 30))
                .unwrap()
                .total_tokens,
            0
        );
        assert_eq!(
            session
                .observe("hermes", &snapshot(10, 2))
                .unwrap()
                .total_tokens,
            12
        );
        assert!(session.observe("hermes", &json!({})).is_none());
        assert!(session.observe("hermes", &snapshot(200, 40)).is_none());
        assert_eq!(
            session
                .observe("hermes", &snapshot(210, 45))
                .unwrap()
                .total_tokens,
            15
        );
        let mut gemini = SessionUsage::default();
        assert_eq!(
            gemini
                .observe("gemini_cli", &snapshot(100, 20))
                .unwrap()
                .total_tokens,
            120
        );
        assert_eq!(
            gemini
                .observe("gemini_cli", &snapshot(100, 20))
                .unwrap()
                .total_tokens,
            120
        );
    }

    #[test]
    fn acp_prompt_usage_is_optional_and_does_not_double_count_reasoning_or_cache() {
        let value = json!({"usage":{"inputTokens":100,"outputTokens":20,"totalTokens":120,"thoughtTokens":10,"cachedReadTokens":80}});
        let usage = prompt_usage(&value).unwrap();
        assert_eq!(usage.total_tokens, 120);
        assert_eq!(usage.thinking_tokens, Some(10));
        assert_eq!(usage.cache_read_tokens, Some(80));
        assert_eq!(
            prompt_usage(&json!({"usage":{"input_tokens":3,"output_tokens":4,"total_tokens":7}}))
                .unwrap()
                .total_tokens,
            7
        );
        for value in [
            json!({}),
            json!({"usage":{"used":100,"size":200,"cost":{"amount":1}}}),
            json!({"usage":{"inputTokens":-1,"outputTokens":2,"totalTokens":1}}),
        ] {
            assert!(prompt_usage(&value).is_none());
        }
    }

    #[test]
    fn hermes_account_quota_preserves_unknowns_fractional_percentages_and_identity() {
        let value = json!({ "provider": "openai-codex", "plan": "Plus", "windows": [
            { "label": "Session", "used_percent": 0.5, "resets_at": "2026-10-11T12:00:00Z" },
            { "label": "Week", "used_percent": null }, { "label": "Other" }
        ] });
        let snapshot = project(&value, "openai-codex").unwrap();
        assert_eq!(snapshot.windows[0].used_percent, Some(0.5));
        assert!(snapshot.windows[1].used_percent.is_none());
        assert!(snapshot.windows[2].used_percent.is_none());
        assert!(project(&value, "anthropic").is_err());
        let unavailable = project(&json!({"provider":"openai-codex", "unavailable_reason":"expired", "windows":[{"used_percent":0}]}), "openai-codex").unwrap();
        assert_eq!(unavailable.status, "unavailable");
        assert!(unavailable.windows.is_empty());
    }

    #[test]
    fn hermes_usage_follows_encoded_provider_without_rewriting_custom_launches() {
        let mut launch = ExternalAgentLaunch::default();
        assert_eq!(
            hermes_usage_args(&launch, "anthropic:claude-sonnet-5-5")
                .unwrap()
                .0,
            ["usage", "--json", "--provider", "anthropic"]
        );
        for model in [
            "claude-sonnet-5-5",
            "custom:work:model",
            "a&b:model",
            "anthropic:",
        ] {
            assert!(hermes_usage_args(&launch, model).is_none());
        }
        launch.args = Some(vec!["--profile".into(), "other".into(), "acp".into()]);
        assert!(hermes_usage_args(&launch, "anthropic:claude").is_none());
    }

    #[tokio::test]
    async fn quota_probe_cannot_block_on_noisy_adapter_stderr() {
        let mut command = tokio::process::Command::new("python");
        command.args(["-c", "import sys;sys.stderr.write('x'*1048576);sys.stderr.flush();print('{\"provider\":\"anthropic\"}')"])
            .stdout(Stdio::piped()).stderr(Stdio::piped());
        assert_eq!(
            read_command(command, Duration::from_secs(5)).await.unwrap()["provider"],
            "anthropic"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn account_probe_bounds_output_and_terminates_stalled_processes() {
        let mut valid = tokio::process::Command::new("sh");
        valid
            .args(["-c", "printf '{\"provider\":\"anthropic\"}'"])
            .stdout(Stdio::piped());
        assert_eq!(
            read_command(valid, Duration::from_secs(2)).await.unwrap()["provider"],
            "anthropic"
        );
        let mut stalled = tokio::process::Command::new("sh");
        stalled.args(["-c", "sleep 30"]).stdout(Stdio::piped());
        assert!(read_command(stalled, Duration::from_millis(100))
            .await
            .unwrap_err()
            .to_string()
            .contains("timed out"));
        let mut noisy = tokio::process::Command::new("sh");
        noisy
            .args(["-c", "head -c 262145 /dev/zero"])
            .stdout(Stdio::piped());
        assert!(read_command(noisy, Duration::from_secs(2))
            .await
            .unwrap_err()
            .to_string()
            .contains("too large"));
    }
}
