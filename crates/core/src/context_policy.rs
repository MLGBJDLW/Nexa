//! User context preferences, scoped to the exact provider endpoint and model.
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{db::Database, error::CoreError};

pub const DEFAULT_COMPACT_PERCENT: u8 = 90;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelContextPolicy {
    pub context_window: Option<u32>,
    pub auto_compact_percent: Option<u8>,
}

impl ModelContextPolicy {
    pub fn compact_percent(&self) -> u8 {
        self.auto_compact_percent
            .unwrap_or(DEFAULT_COMPACT_PERCENT)
            .clamp(60, 95)
    }

    pub fn validate(&self, model_limit: Option<u32>) -> Result<(), CoreError> {
        if self
            .auto_compact_percent
            .is_some_and(|value| !(60..=95).contains(&value))
        {
            return Err(CoreError::InvalidInput(
                "Auto-compaction must be between 60% and 95% of the input budget".into(),
            ));
        }
        if let Some(window) = self.context_window {
            if !(8_192..=16_777_216).contains(&window)
                || model_limit.is_some_and(|limit| window > limit)
            {
                return Err(CoreError::InvalidInput("Context budget must be at least 8192 tokens and cannot exceed the verified model limit".into()));
            }
        }
        Ok(())
    }
}

fn policy_key(provider: &str, base_url: Option<&str>, model: &str) -> String {
    let endpoint = crate::model_catalog::resolve_or_derive_endpoint_id("text", provider, base_url);
    format!(
        "model_context_policy_v1:{}",
        serde_json::json!([endpoint, model.trim().to_lowercase()])
    )
}

impl Database {
    pub fn load_model_context_policy(
        &self,
        provider: &str,
        base_url: Option<&str>,
        model: &str,
    ) -> Result<Option<ModelContextPolicy>, CoreError> {
        let conn = self.conn();
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='app_config')",
            [],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM app_config WHERE key = ?1",
                [policy_key(provider, base_url, model)],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(|json| serde_json::from_str(&json).map_err(CoreError::from))
            .transpose()
    }

    pub fn save_model_context_policy(
        &self,
        provider: &str,
        base_url: Option<&str>,
        model: &str,
        policy: &ModelContextPolicy,
    ) -> Result<(), CoreError> {
        if model.trim().is_empty() {
            return Err(CoreError::InvalidInput("Select a model first".into()));
        }
        let model_limit = crate::provider_catalog::resolve_endpoint_model_context_window(
            provider, base_url, model, None,
        )
        .capacity_tokens;
        policy.validate(model_limit)?;
        let conn = self.conn();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS app_config (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL, updated_at TEXT NOT NULL DEFAULT (datetime('now')))")?;
        conn.execute("INSERT INTO app_config (key,value,updated_at) VALUES (?1,?2,datetime('now')) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at", params![policy_key(provider, base_url, model), serde_json::to_string(policy)?])?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelContextPolicySnapshot {
    pub model: String,
    pub policy: ModelContextPolicy,
    pub model_limit: Option<u32>,
    pub effective_context_window: Option<u32>,
    pub context_authority: crate::conversation::memory::ContextWindowAuthority,
    pub response_token_limit: u32,
    pub response_reserve_target: u32,
    pub response_reserve_is_automatic: bool,
    pub response_reserve: u32,
    pub default_compact_percent: u8,
    pub safety_reserve: u32,
    pub prompt_budget: Option<u32>,
    pub trigger_tokens: Option<u32>,
    pub managed_by_provider: bool,
}

pub fn policy_snapshot(
    db: &Database,
    config: &crate::conversation::AgentConfig,
) -> Result<ModelContextPolicySnapshot, CoreError> {
    let policy = db
        .load_model_context_policy(&config.provider, config.base_url.as_deref(), &config.model)?
        .unwrap_or(ModelContextPolicy {
            context_window: config
                .context_window
                .and_then(|value| u32::try_from(value).ok()),
            auto_compact_percent: None,
        });
    let natural = crate::provider_catalog::resolve_endpoint_model_context_window(
        &config.provider,
        config.base_url.as_deref(),
        &config.model,
        None,
    );
    let resolved = crate::provider_catalog::resolve_endpoint_model_context_window(
        &config.provider,
        config.base_url.as_deref(),
        &config.model,
        policy.context_window,
    );
    let mut agent = crate::agent::AgentConfig {
        // Chat/Plan retired saved per-request output caps. The UI projection
        // must use the same automatic reserve as the desktop executor, even
        // when an older profile still contains a non-null legacy value.
        max_tokens: None,
        provider_type: Some(crate::provider_registry::provider_type_for_parts(
            &config.provider,
            config.base_url.as_deref(),
        )),
        context_window_resolution: Some(natural),
        catalog_limits_authoritative: Some(
            crate::provider_catalog::endpoint_model_catalog_limits_are_authoritative(
                &config.provider,
                config.base_url.as_deref(),
                &config.model,
            ),
        ),
        ..Default::default()
    };
    let response_token_limit = agent.resolved_response_token_limit(&config.model);
    let response_reserve_target = agent.resolved_max_response_tokens(&config.model);
    agent.context_window_resolution = Some(resolved);
    let response_reserve = agent.resolved_max_response_tokens(&config.model);
    let safety_reserve = resolved
        .capacity_tokens
        .map(crate::conversation::memory::context_safety_buffer)
        .unwrap_or(0);
    let prompt_budget = resolved.capacity_tokens.map(|capacity| {
        capacity
            .saturating_sub(response_reserve)
            .saturating_sub(safety_reserve)
    });
    Ok(ModelContextPolicySnapshot {
        model: config.model.clone(),
        model_limit: natural.capacity_tokens,
        effective_context_window: resolved.capacity_tokens,
        context_authority: resolved.authority,
        response_token_limit,
        response_reserve_target,
        response_reserve_is_automatic: agent.max_tokens.is_none(),
        response_reserve,
        default_compact_percent: DEFAULT_COMPACT_PERCENT,
        safety_reserve,
        prompt_budget,
        trigger_tokens: prompt_budget
            .map(|budget| (u64::from(budget) * u64::from(policy.compact_percent()) / 100) as u32),
        managed_by_provider: matches!(config.provider.as_str(), "github_copilot" | "openai_codex"),
        policy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_distinguishes_output_capacity_from_planning_reserve() {
        let db = Database::open_memory().unwrap();
        let mut config: crate::conversation::AgentConfig =
            serde_json::from_value(serde_json::json!({
                "id":"test", "name":"DeepSeek", "provider":"deep_seek", "apiKey":"",
                "baseUrl":"https://api.deepseek.com", "model":"deepseek-flash", "isDefault":true,
                "createdAt":"", "updatedAt":""
            }))
            .unwrap();
        let automatic = policy_snapshot(&db, &config).unwrap();
        assert_eq!(automatic.model_limit, Some(1_000_000));
        assert_eq!(automatic.response_token_limit, 384_000);
        assert_eq!(automatic.response_reserve, 32_768);
        assert_eq!(automatic.trigger_tokens, Some(863_136));
        assert!(automatic.response_reserve_is_automatic);

        config.max_tokens = Some(120_000);
        let legacy = policy_snapshot(&db, &config).unwrap();
        assert_eq!(legacy.response_token_limit, automatic.response_token_limit);
        assert_eq!(legacy.response_reserve, automatic.response_reserve);
        assert!(legacy.response_reserve_is_automatic);
        assert_eq!(legacy.trigger_tokens, automatic.trigger_tokens);
    }

    #[test]
    fn model_policies_survive_reopen_and_never_cross_endpoints_or_models() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("context.db");
        let policy = ModelContextPolicy {
            context_window: Some(128_000),
            auto_compact_percent: Some(65),
        };
        {
            let db = Database::new(&path).unwrap();
            db.save_model_context_policy(
                "open_ai",
                Some("https://api.openai.com/v1"),
                "gpt-6-sol",
                &policy,
            )
            .unwrap();
        }
        let db = Database::new(&path).unwrap();
        assert_eq!(
            db.load_model_context_policy(
                "open_ai",
                Some("https://api.openai.com/v1/"),
                "gpt-6-sol"
            )
            .unwrap(),
            Some(policy)
        );
        assert!(db
            .load_model_context_policy("open_ai", Some("https://private.example/v1"), "gpt-6-sol")
            .unwrap()
            .is_none());
        assert!(db
            .load_model_context_policy("open_ai", None, "gpt-6-luna")
            .unwrap()
            .is_none());
        db.save_model_context_policy(
            "open_ai",
            Some("https://api.openai.com/v1"),
            "gpt-6-sol",
            &ModelContextPolicy::default(),
        )
        .unwrap();
        assert_eq!(
            db.load_model_context_policy("open_ai", Some("https://api.openai.com/v1"), "gpt-6-sol")
                .unwrap(),
            Some(ModelContextPolicy::default())
        );
    }

    #[test]
    fn rejects_impossible_capacity_and_unsafe_thresholds() {
        assert!(ModelContextPolicy {
            context_window: Some(256_000),
            auto_compact_percent: None
        }
        .validate(Some(128_000))
        .is_err());
        for percent in [0, 55, 59, 96, 100] {
            assert!(ModelContextPolicy {
                context_window: None,
                auto_compact_percent: Some(percent)
            }
            .validate(None)
            .is_err());
        }
    }
}
