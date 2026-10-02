//! One analysis cache for immutable request snapshots.
//!
//! Revision identity is process-local and never substitutes for the stable
//! hashes persisted in prompt-cache diagnostics. Old payloads are analyzed
//! once; appending a tail only analyzes that tail. Route changes reset the
//! tokenizer-dependent cache, and unreachable revisions are released.

use std::collections::{hash_map::DefaultHasher, BTreeMap, HashMap, HashSet};
use std::hash::Hasher;
use std::sync::Arc;

use super::{context, prompt_cache, AgentExecutor};
use crate::conversation::memory::{estimate_message_tokens_for_model, estimate_tokens_for_model};
use crate::llm::prompt_cache::PromptCacheProfileKey;
use crate::llm::{Message, Role, ToolDefinition};

pub(super) const PREFIX_WINDOWS: [u32; 3] = [1_024, 4_096, 16_384];

#[derive(Debug)]
pub(super) struct MessageMetrics {
    pub(super) revision: u64,
    pub(super) role: Role,
    pub(super) segments: BTreeMap<&'static str, u32>,
    pub(super) base_tokens: u32,
    pub(super) cache_tokens: u32,
    pub(super) hash: u64,
    pub(super) fingerprint: prompt_cache::PromptCacheMessageFingerprint,
    pub(super) serialized: String,
    pub(super) text_hash: u64,
}

impl MessageMetrics {
    fn analyze(model: &str, message: &Message) -> Self {
        let mut segments = BTreeMap::new();
        context::add_message_context_tokens(&mut segments, model, message);
        // Tool results already include every part and tool call. Reuse that
        // exact estimate instead of tokenizing their often very large text a
        // second time for cache diagnostics.
        let base_tokens = if message.role == Role::Tool {
            segments.values().copied().sum()
        } else {
            estimate_message_tokens_for_model(model, message)
        };
        let cache_tokens =
            base_tokens.saturating_add(message.reasoning_content.as_deref().map_or(0, |text| {
                segments
                    .get("thinking")
                    .copied()
                    .unwrap_or_else(|| estimate_tokens_for_model(model, text))
            }));
        let serialized = prompt_cache::serialized_message_for_hash(message);
        Self {
            revision: message.revision(),
            role: message.role.clone(),
            segments,
            base_tokens,
            cache_tokens,
            hash: prompt_cache::hash_text(&serialized),
            fingerprint: prompt_cache::message_fingerprint(message),
            serialized,
            text_hash: prompt_cache::hash_text(&message.text_content()),
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ToolMetrics {
    definitions: Vec<ToolDefinition>,
    pub(super) segments: BTreeMap<&'static str, u32>,
    pub(super) tokens: u32,
    pub(super) hash: u64,
    pub(super) hashes: Vec<u64>,
    pub(super) names: Vec<String>,
}

impl ToolMetrics {
    fn matches(&self, tools: &[ToolDefinition]) -> bool {
        self.definitions.len() == tools.len()
            && self.definitions.iter().zip(tools).all(|(old, new)| {
                old.name == new.name
                    && old.description == new.description
                    && old.parameters == new.parameters
            })
    }

    fn analyze(model: &str, tools: &[ToolDefinition]) -> Self {
        let mut segments = BTreeMap::new();
        for tool in tools {
            let kind = if context::is_mcp_tool_definition(tool) {
                "mcp"
            } else {
                "tools"
            };
            *segments.entry(kind).or_default() +=
                context::estimate_tool_definition_tokens_for_model(model, tool);
        }
        Self {
            definitions: tools.to_vec(),
            tokens: segments.values().copied().sum(),
            segments,
            hash: prompt_cache::tool_schema_hash(tools),
            hashes: prompt_cache::individual_tool_schema_hashes(tools),
            names: tools.iter().map(|tool| tool.name.clone()).collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ContextMetricsSnapshot {
    pub(super) messages: Vec<Arc<MessageMetrics>>,
    pub(super) tools: Arc<ToolMetrics>,
    pub(super) prefix_hashes: [u64; 3],
}

#[derive(Debug)]
struct ProviderObservation {
    revisions: Vec<u64>,
    tool_hash: u64,
    actual_tokens: u32,
}

impl ContextMetricsSnapshot {
    pub(super) fn breakdown(&self, actual_tokens: Option<u32>) -> context::ContextUsageBreakdown {
        let mut segments = self.tools.segments.clone();
        for message in &self.messages {
            for (kind, tokens) in &message.segments {
                *segments.entry(kind).or_default() += tokens;
            }
        }
        context::breakdown_from_segments(segments, actual_tokens)
    }
}

#[derive(Debug, Default)]
struct PrefixState {
    revisions: Vec<u64>,
    hasher: DefaultHasher,
    tokens: u32,
    complete: bool,
}

impl PrefixState {
    fn hash(&mut self, messages: &[Arc<MessageMetrics>], budget: u32) -> u64 {
        if self.revisions.len() > messages.len()
            || self
                .revisions
                .iter()
                .zip(messages)
                .any(|(old, new)| *old != new.revision)
        {
            *self = Self::default();
        }
        if !self.complete {
            for message in &messages[self.revisions.len()..] {
                if self.tokens >= budget {
                    self.complete = true;
                    break;
                }
                self.revisions.push(message.revision);
                if self.tokens.saturating_add(message.cache_tokens) <= budget {
                    self.hasher.write(message.serialized.as_bytes());
                    self.tokens = self.tokens.saturating_add(message.cache_tokens);
                } else {
                    let keep_chars = budget.saturating_sub(self.tokens).saturating_mul(4) as usize;
                    let byte_end = message
                        .serialized
                        .char_indices()
                        .nth(keep_chars)
                        .map_or(message.serialized.len(), |(index, _)| index);
                    self.hasher.write(message.serialized[..byte_end].as_bytes());
                    self.complete = true;
                    break;
                }
            }
        }
        // str::hash writes the UTF-8 bytes followed by this terminator. Keep
        // the appendable state before the terminator to retain old persisted
        // prefix fingerprints byte for byte.
        let mut finalized = self.hasher.clone();
        finalized.write_u8(0xff);
        finalized.finish()
    }
}

#[derive(Debug, Default)]
pub(super) struct ContextMetrics {
    key: Option<PromptCacheProfileKey>,
    model: String,
    messages: HashMap<u64, Arc<MessageMetrics>>,
    tools: Option<Arc<ToolMetrics>>,
    prefixes: [PrefixState; 3],
    observation: Option<ProviderObservation>,
    #[cfg(test)]
    analyzed_messages: usize,
    #[cfg(test)]
    analyzed_tool_surfaces: usize,
    #[cfg(test)]
    analyzed_tool_messages: usize,
}

impl ContextMetrics {
    pub(super) fn snapshot(
        &mut self,
        key: PromptCacheProfileKey,
        model: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> ContextMetricsSnapshot {
        if self.key.as_ref() != Some(&key) || self.model != model {
            self.messages.clear();
            self.tools = None;
            self.prefixes = Default::default();
            self.observation = None;
            self.key = Some(key);
            self.model = model.to_string();
        }
        let reachable = messages
            .iter()
            .map(Message::revision)
            .collect::<HashSet<_>>();
        self.messages
            .retain(|revision, _| reachable.contains(revision));
        let analyzed = messages
            .iter()
            .map(|message| {
                Arc::clone(self.messages.entry(message.revision()).or_insert_with(|| {
                    #[cfg(test)]
                    {
                        self.analyzed_messages += 1;
                        self.analyzed_tool_messages += usize::from(message.role == Role::Tool);
                    }
                    Arc::new(MessageMetrics::analyze(model, message))
                }))
            })
            .collect::<Vec<_>>();
        if self
            .tools
            .as_ref()
            .is_none_or(|existing| !existing.matches(tools))
        {
            #[cfg(test)]
            {
                self.analyzed_tool_surfaces += 1;
            }
            self.tools = Some(Arc::new(ToolMetrics::analyze(model, tools)));
        }
        let prefix_hashes = std::array::from_fn(|index| {
            self.prefixes[index].hash(&analyzed, PREFIX_WINDOWS[index])
        });
        ContextMetricsSnapshot {
            messages: analyzed,
            tools: Arc::clone(self.tools.as_ref().expect("tool metrics initialized")),
            prefix_hashes,
        }
    }

    fn input_tokens(&self, snapshot: &ContextMetricsSnapshot) -> u32 {
        let estimated = snapshot.breakdown(None).total_tokens;
        let Some(observed) = self.observation.as_ref().filter(|observed| {
            observed.tool_hash == snapshot.tools.hash
                && observed.revisions.len() <= snapshot.messages.len()
                && observed
                    .revisions
                    .iter()
                    .zip(&snapshot.messages)
                    .all(|(old, new)| *old == new.revision)
        }) else {
            return estimated;
        };
        let tail = snapshot.messages[observed.revisions.len()..]
            .iter()
            .flat_map(|message| message.segments.values())
            .copied()
            .sum::<u32>();
        estimated.max(observed.actual_tokens.saturating_add(tail))
    }

    fn observe(&mut self, snapshot: &ContextMetricsSnapshot, actual_tokens: Option<u32>) {
        self.observation = actual_tokens
            .filter(|tokens| *tokens > 0)
            .map(|actual_tokens| ProviderObservation {
                revisions: snapshot
                    .messages
                    .iter()
                    .map(|message| message.revision)
                    .collect(),
                tool_hash: snapshot.tools.hash,
                actual_tokens,
            });
    }

    #[cfg(test)]
    pub(super) fn analyzed_tool_message_count(&self) -> usize {
        self.analyzed_tool_messages
    }
}

impl AgentExecutor {
    pub(super) fn context_metrics_snapshot(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> ContextMetricsSnapshot {
        self.context_metrics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .snapshot(
                self.provider.prompt_cache_profile(model).key,
                model,
                messages,
                tools,
            )
    }

    pub(super) fn context_usage_breakdown(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        actual_tokens: Option<u32>,
    ) -> context::ContextUsageBreakdown {
        self.context_metrics_snapshot(model, messages, tools)
            .breakdown(actual_tokens)
    }

    pub(super) fn context_input_tokens(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> u32 {
        let mut metrics = self
            .context_metrics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let snapshot = metrics.snapshot(
            self.provider.prompt_cache_profile(model).key,
            model,
            messages,
            tools,
        );
        metrics.input_tokens(&snapshot)
    }

    /// Call only for an accepted request on the same concrete route and with
    /// no omitted replay units. A fallback sample cannot calibrate its primary.
    pub(super) fn observe_context_input_tokens(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        actual_tokens: Option<u32>,
    ) {
        let mut metrics = self
            .context_metrics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let snapshot = metrics.snapshot(
            self.provider.prompt_cache_profile(model).key,
            model,
            messages,
            tools,
        );
        metrics.observe(&snapshot, actual_tokens);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::prompt_cache::{resolve_prompt_cache_profile, PromptCacheApiStyle};
    use crate::llm::ProviderType;

    fn key(model: &str) -> PromptCacheProfileKey {
        resolve_prompt_cache_profile(
            ProviderType::OpenAi,
            None,
            PromptCacheApiStyle::OpenAiCompatible,
            model,
        )
        .key
    }

    fn tool() -> ToolDefinition {
        ToolDefinition {
            name: "read_file".into(),
            description: "Read a file".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}}),
        }
    }

    #[test]
    fn append_only_requests_analyze_only_new_messages_and_changed_tool_surfaces() {
        let model = "gpt-4o";
        let mut metrics = ContextMetrics::default();
        let mut messages = vec![
            Message::text(Role::System, "rules"),
            Message::text(Role::User, "inspect this"),
            Message::text_with_name(Role::Tool, "large evidence ".repeat(20_000), "call-1"),
        ];
        let mut tools = vec![tool()];
        let first = metrics.snapshot(key(model), model, &messages, &tools);
        let second = metrics.snapshot(key(model), model, &messages.clone(), &tools);
        assert_eq!(first.breakdown(None), second.breakdown(None));
        assert_eq!(metrics.analyzed_messages, 3);
        assert_eq!(metrics.analyzed_tool_surfaces, 1);
        assert!(Arc::ptr_eq(&first.messages[2], &second.messages[2]));
        messages.push(Message::text(Role::Assistant, "next action"));
        metrics.snapshot(key(model), model, &messages, &tools);
        assert_eq!(metrics.analyzed_messages, 4);
        tools[0].description.push_str(" with updated semantics");
        metrics.snapshot(key(model), model, &messages, &tools);
        assert_eq!(metrics.analyzed_messages, 4);
        assert_eq!(metrics.analyzed_tool_surfaces, 2);
    }

    #[test]
    fn mutation_recovery_model_and_route_changes_invalidate_the_right_entries() {
        let model = "gpt-4o";
        let mut metrics = ContextMetrics::default();
        let mut messages = vec![Message::text(Role::User, "old")];
        metrics.snapshot(key(model), model, &messages, &[]);
        messages[0].parts = vec![crate::llm::ContentPart::Text { text: "new".into() }];
        metrics.snapshot(key(model), model, &messages, &[]);
        assert_eq!(metrics.analyzed_messages, 2);
        assert_eq!(metrics.messages.len(), 1);
        messages = serde_json::from_value(serde_json::to_value(messages).unwrap()).unwrap();
        metrics.snapshot(key(model), model, &messages, &[]);
        assert_eq!(metrics.analyzed_messages, 3);
        metrics.snapshot(key("claude-sonnet-4"), "claude-sonnet-4", &messages, &[]);
        assert_eq!(metrics.analyzed_messages, 4);
        let mut alternate = key("claude-sonnet-4");
        alternate.endpoint_id = "another-endpoint".into();
        metrics.snapshot(alternate, "claude-sonnet-4", &messages, &[]);
        assert_eq!(metrics.analyzed_messages, 5);
    }

    #[test]
    fn provider_feedback_applies_only_to_an_unchanged_prefix_and_tool_surface() {
        let model = "gpt-4o";
        let mut metrics = ContextMetrics::default();
        let mut messages = vec![Message::text(Role::User, "request")];
        let first = metrics.snapshot(key(model), model, &messages, &[]);
        metrics.observe(&first, Some(5_000));
        messages.push(Message::text(Role::Assistant, "a new tail"));
        let next = metrics.snapshot(key(model), model, &messages, &[]);
        assert_eq!(
            metrics.input_tokens(&next),
            5_000 + next.messages[1].segments.values().sum::<u32>()
        );
        messages[0] = Message::text(Role::User, "changed request");
        let changed = metrics.snapshot(key(model), model, &messages, &[]);
        assert_eq!(
            metrics.input_tokens(&changed),
            changed.breakdown(None).total_tokens
        );
        metrics.observe(&changed, Some(6_000));
        let changed_tools = metrics.snapshot(key(model), model, &messages, &[tool()]);
        assert_eq!(
            metrics.input_tokens(&changed_tools),
            changed_tools.breakdown(None).total_tokens
        );
    }

    #[test]
    fn cached_diagnostics_match_the_legacy_algorithm_for_append_mutation_and_recovery() {
        let model = "gpt-4o";
        let profile = resolve_prompt_cache_profile(
            ProviderType::OpenAi,
            None,
            PromptCacheApiStyle::OpenAiCompatible,
            model,
        );
        let mut metrics = ContextMetrics::default();
        let mut messages = vec![
            Message::text(
                Role::System,
                "policy\n## Active task plan\ninspect\n## User long-term memory\npreferences",
            ),
            Message::text(Role::User, "中文 request"),
            Message::text_with_name(Role::Tool, "中文 evidence ".repeat(1_500), "call-1"),
        ];
        let tools = vec![tool()];
        for step in 0..5 {
            let snapshot = metrics.snapshot(key(model), model, &messages, &tools);
            assert_eq!(
                snapshot.breakdown(None),
                context::estimate_context_usage_breakdown_for_model(model, &messages, &tools, None)
            );
            assert_eq!(
                snapshot.breakdown(Some(51)),
                context::estimate_context_usage_breakdown_for_model(
                    model,
                    &messages,
                    &tools,
                    Some(51)
                )
            );
            assert_eq!(
                prompt_cache::snapshot_from_metrics(
                    Some(ProviderType::OpenAi),
                    profile.clone(),
                    model,
                    &snapshot
                ),
                prompt_cache::snapshot_for_profile(
                    Some(ProviderType::OpenAi),
                    profile.clone(),
                    model,
                    &messages,
                    &tools
                )
            );
            match step {
                0 => messages.push(Message::text(Role::Assistant, "continue")),
                1 => messages[1].parts.push(crate::llm::ContentPart::Image {
                    media_type: "image/png".into(),
                    data: "AAAA".repeat(1_000),
                }),
                2 => {
                    messages =
                        serde_json::from_value(serde_json::to_value(&messages).unwrap()).unwrap()
                }
                3 => {
                    messages.remove(2);
                }
                _ => {}
            }
        }
    }

    #[test]
    #[ignore = "manual runtime performance probe; no timing assertion"]
    fn context_metrics_large_history_work_probe() {
        let model = "gpt-4o";
        let payload = "source evidence alpha beta gamma delta epsilon;\n".repeat(350);
        let mut messages = (0..128)
            .map(|index| {
                Message::text_with_name(Role::Tool, payload.clone(), format!("call-{index}"))
            })
            .collect::<Vec<_>>();
        let mut metrics = ContextMetrics::default();
        let start = std::time::Instant::now();
        let cold = metrics.snapshot(key(model), model, &messages, &[]);
        let cold_elapsed = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..5 {
            std::hint::black_box(metrics.snapshot(key(model), model, &messages, &[]));
        }
        let warm_elapsed = start.elapsed();
        let analyzed_before_append = metrics.analyzed_messages;
        messages.push(Message::text(Role::User, "new tail"));
        let start = std::time::Instant::now();
        std::hint::black_box(metrics.snapshot(key(model), model, &messages, &[]));
        let append_elapsed = start.elapsed();
        assert_eq!(analyzed_before_append, 128);
        assert_eq!(metrics.analyzed_messages, 129);
        println!("context_metrics profile={} payload_bytes={} old_messages=128 cold_ms={:.3} five_warm_ms={:.3} append_ms={:.3} analyzed_before_append={} analyzed_after_append={} prompt_tokens={}",
            if cfg!(debug_assertions) { "debug" } else { "release" }, payload.len() * 128,
            cold_elapsed.as_secs_f64() * 1_000.0, warm_elapsed.as_secs_f64() * 1_000.0,
            append_elapsed.as_secs_f64() * 1_000.0, analyzed_before_append, metrics.analyzed_messages,
            cold.breakdown(None).total_tokens);
    }
}
