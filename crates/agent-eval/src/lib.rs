//! Task outcomes through the real Nexa executor and file tools.
//!
//! Scripted mode verifies the harness/runtime without a paid account. Live mode
//! measures the configured model; these result classes are never interchangeable.

mod provenance;
mod provider;

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nexa_core::agent::{AgentConfig, AgentEvent, AgentExecutor};
use nexa_core::approval::{ApprovalDecision, ToolApprovalMode};
use nexa_core::db::Database;
use nexa_core::llm::{ContentPart, ProviderConfig, ProviderStreamingConfig, ProviderType};
use nexa_core::tools::{
    create_file_tool::CreateFileTool, edit_file_tool::EditFileTool, file_tool::FileTool,
    list_dir_tool::ListDirTool, read_files_tool::ReadFilesTool, tool_search_tool::ToolSearchTool,
    ToolRegistry,
};
use nexa_core::workspace::Workspace;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use provider::{Calls, MeteredProvider, ProviderInvocation, ScriptedProvider};

pub type EvalResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReferenceAction {
    pub tool: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputImage {
    pub data: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub category: String,
    pub prompt: String,
    pub files: BTreeMap<String, String>,
    pub oracle: String,
    pub expected_files: Vec<String>,
    #[serde(default)]
    pub require_approval: bool,
    pub reference_actions: Vec<ReferenceAction>,
    pub reference_answer: String,
    #[serde(default)]
    pub input_images: Vec<InputImage>,
    #[serde(default)]
    pub required_tools: Vec<String>,
    #[serde(default)]
    pub min_tool_calls: usize,
    #[serde(default)]
    pub required_tool_counts: BTreeMap<String, usize>,
    #[serde(default)]
    pub context_window_tokens: Option<u32>,
    #[serde(default)]
    pub compaction_summary: Option<String>,
    #[serde(default)]
    pub require_compaction: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvalMode {
    #[default]
    Scripted,
    Live,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenPrices {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cache_read_per_million: f64,
    pub cache_creation_per_million: f64,
    pub currency: String,
    pub source: String,
}

impl TokenPrices {
    fn estimate(
        &self,
        calls: &[ProviderInvocation],
        api_style: nexa_core::llm::reasoning_profile::ReasoningApiStyle,
    ) -> Option<f64> {
        if calls.iter().any(|call| !call.usage_reported) {
            return None;
        }
        Some(
            calls
                .iter()
                .map(|call| {
                    let read = call.cache_read_tokens.unwrap_or(0);
                    let write = call.cache_creation_tokens.unwrap_or(0);
                    let uncached = if api_style
                        == nexa_core::llm::reasoning_profile::ReasoningApiStyle::AnthropicMessages
                    {
                        // Anthropic adapter exposes disjoint uncached/read/write input counters.
                        call.prompt_tokens
                    } else {
                        call.prompt_tokens
                            .saturating_sub(read)
                            .saturating_sub(write)
                    };
                    (f64::from(uncached) * self.input_per_million
                        + f64::from(call.completion_tokens) * self.output_per_million
                        + f64::from(read) * self.cache_read_per_million
                        + f64::from(write) * self.cache_creation_per_million)
                        / 1_000_000.0
                })
                .sum(),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct EvalConfig {
    pub model: String,
    pub provider: ProviderType,
    pub base_url: Option<String>,
    pub repetitions: u32,
    pub max_tokens_per_task: u32,
    pub timeout_seconds: u64,
    pub reasoning_enabled: Option<bool>,
    pub prices: Option<TokenPrices>,
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            model: "scripted-eval".into(),
            provider: ProviderType::Custom,
            base_url: None,
            repetitions: 1,
            max_tokens_per_task: 24_000,
            timeout_seconds: 120,
            reasoning_enabled: None,
            prices: None,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEvidence {
    pub path: String,
    pub digest: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskReport {
    pub task_id: String,
    pub category: String,
    pub repetition: u32,
    pub passed: bool,
    pub oracle_passed: bool,
    pub runtime_evidence_passed: bool,
    pub completed_tools: BTreeMap<String, usize>,
    pub compactions: usize,
    pub input_images: usize,
    pub observed_input_images: usize,
    pub context_window_tokens: Option<u32>,
    pub error: Option<String>,
    pub oracle_diagnostic: String,
    /// Complete task time: workspace preparation, executor, oracle and evidence.
    pub elapsed_ms: u64,
    pub preparation_ms: u64,
    pub executor_elapsed_ms: u64,
    pub oracle_ms: u64,
    pub provider_invocation_ms: u64,
    /// Wall-clock remainder, not a CPU measurement: includes tool and scheduling work.
    pub orchestration_and_tools_ms: u64,
    pub provider_invocations: Vec<ProviderInvocation>,
    /// Adapter-internal HTTP retries are not exposed at the provider trait boundary.
    pub physical_attempt_count: Option<usize>,
    pub cost_scope: &'static str,
    pub tool_calls: usize,
    pub available_tools: Vec<String>,
    pub failed_tool_calls: usize,
    pub approvals: usize,
    pub reported_usage_cost: Option<f64>,
    pub answer_digest: String,
    pub outputs: Vec<FileEvidence>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuiteReport {
    pub schema_version: u32,
    pub mode: EvalMode,
    pub score_kind: &'static str,
    pub source_sha: String,
    pub source_dirty: bool,
    pub source_fingerprint: String,
    pub compiled_source_sha: String,
    pub compiled_source_dirty: bool,
    pub compiled_source_fingerprint: String,
    pub corpus_digest: String,
    pub node_version: String,
    pub target_triple: &'static str,
    pub build_profile: &'static str,
    pub rustc_version: &'static str,
    pub model: String,
    pub provider: ProviderType,
    pub api_style: nexa_core::llm::reasoning_profile::ReasoningApiStyle,
    pub reasoning_enabled: Option<bool>,
    pub provider_endpoint_id: String,
    pub repetitions: u32,
    pub max_tokens_per_task: u32,
    pub max_output_tokens_per_request: u32,
    pub timeout_seconds: u64,
    pub prices: Option<TokenPrices>,
    pub passed: usize,
    pub total: usize,
    pub cases: Vec<TaskReport>,
}

pub fn builtin_tasks() -> EvalResult<(Vec<Task>, String)> {
    let bytes = include_bytes!("../../../eval/tasks.json");
    Ok((
        serde_json::from_slice(bytes)?,
        blake3::hash(bytes).to_hex().to_string(),
    ))
}

fn safe_relative(root: &Path, relative: &str) -> EvalResult<PathBuf> {
    let path = Path::new(relative);
    if relative.is_empty()
        || relative.contains([':', '\\'])
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(format!("Task file must be a nonempty relative path: {relative}").into());
    }
    Ok(root.join(path))
}

fn prepare_workspace(task: &Task, root: &Path) -> EvalResult<()> {
    std::fs::create_dir_all(root)?;
    for (relative, content) in &task.files {
        let path = safe_relative(root, relative)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
    }
    Ok(())
}

async fn check_oracle(
    task: &Task,
    outer: &Path,
    workspace: &Path,
    answer: &str,
) -> EvalResult<(bool, String)> {
    // Agent file tools cannot reach the oracle or answer. Node also has read-only
    // access, but imported candidate code shares its VM: this is a quality-test
    // boundary, not a claim of adversarial JavaScript isolation.
    let oracle = outer.join("oracle.mjs");
    let runner = outer.join("oracle-runner.mjs");
    let answer_path = outer.join("answer.txt");
    std::fs::write(&oracle, &task.oracle)?;
    std::fs::write(&answer_path, answer)?;
    let nonce = format!(
        "{}:{:?}",
        outer.display(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?
    );
    let completion = format!("NEXA_ORACLE_COMPLETED:{}", blake3::hash(nonce.as_bytes()));
    let runner_source = include_str!("../../../eval/oracle-runner.mjs").replace(
        "__NEXA_ORACLE_COMPLETION_JSON__",
        &serde_json::to_string(&completion)?,
    );
    std::fs::write(&runner, runner_source)?;
    let mut command = tokio::process::Command::new("node");
    command
        .arg("--permission")
        .arg("--no-addons")
        .arg(format!("--allow-fs-read={}", workspace.display()))
        .arg(format!("--allow-fs-read={}", oracle.display()))
        .arg(format!("--allow-fs-read={}", runner.display()))
        .arg(format!("--allow-fs-read={}", answer_path.display()))
        .arg(&runner)
        .arg(workspace)
        .arg(&answer_path)
        .arg(&oracle)
        .current_dir(workspace)
        .env_clear()
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // Do not expose the live provider credential (or unrelated user variables)
    // to code under evaluation. These two are only process-loader necessities.
    for name in ["PATH", "SystemRoot"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command.spawn()?;
    let mut stderr = child.stderr.take().ok_or("missing oracle stderr")?;
    let diagnostics = tokio::spawn(async move {
        let mut retained = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stderr.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            let keep = read.min(8192_usize.saturating_sub(retained.len()));
            retained.extend_from_slice(&buffer[..keep]);
        }
        Ok::<_, std::io::Error>(retained)
    });
    let mut stdout = child.stdout.take().ok_or("missing oracle stdout")?;
    let completion = format!("\n{completion}\n").into_bytes();
    let completion_read = tokio::spawn(async move {
        let mut completed = false;
        let mut boundary = Vec::with_capacity(4096 + completion.len());
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stdout.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            if completed {
                continue;
            }
            boundary.extend_from_slice(&buffer[..read]);
            completed = boundary
                .windows(completion.len())
                .any(|window| window == completion.as_slice());
            // Only the suffix can combine with the next read to complete a
            // marker. Never retain unbounded stdout, even for noisy candidates.
            let keep = completion.len().saturating_sub(1);
            if boundary.len() > keep {
                boundary.drain(..boundary.len() - keep);
            }
        }
        Ok::<_, std::io::Error>(completed)
    });
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
    if status.is_err() {
        child.kill().await?;
    }
    let retained = diagnostics.await??;
    let completed = completion_read.await??;
    match status {
        Ok(status) => {
            let mut diagnostic = String::from_utf8_lossy(&retained)
                .chars()
                .take(2_000)
                .collect::<String>();
            if !completed {
                diagnostic =
                    format!("Oracle did not reach its trusted completion point. {diagnostic}");
            }
            Ok((status?.success() && completed, diagnostic))
        }
        Err(_) => Ok((false, "oracle deadline exceeded".into())),
    }
}

async fn run_task(
    task: &Task,
    repetition: u32,
    mode: EvalMode,
    config: &EvalConfig,
    api_key: Option<&str>,
) -> EvalResult<TaskReport> {
    let task_started = Instant::now();
    let outer = tempfile::tempdir()?;
    let workspace_path = outer.path().join("workspace");
    prepare_workspace(task, &workspace_path)?;
    let workspace = Workspace::validate(&[workspace_path.to_string_lossy().to_string()])?;
    let workspace_prompt = workspace.prompt();
    let mut registry = ToolRegistry::new().with_workspace(Some(workspace));
    registry.register(Box::new(FileTool));
    registry.register(Box::new(ReadFilesTool));
    registry.register(Box::new(EditFileTool));
    registry.register(Box::new(CreateFileTool));
    registry.register(Box::new(ListDirTool));
    registry.register(Box::new(ToolSearchTool));
    let tool_names = registry
        .definitions()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    if task
        .reference_actions
        .iter()
        .any(|action| !tool_names.contains(&action.tool))
    {
        return Err(format!("Task {} references an unregistered tool", task.id).into());
    }
    let inner: Box<dyn nexa_core::llm::LlmProvider> = match mode {
        EvalMode::Scripted => Box::new(ScriptedProvider::new(task.clone())),
        EvalMode::Live => nexa_core::llm::create_provider(ProviderConfig {
            provider_type: config.provider,
            base_url: config.base_url.clone(),
            api_key: api_key.map(str::to_owned),
            org_id: None,
            timeout_secs: Some(config.timeout_seconds),
            streaming: ProviderStreamingConfig::default(),
        })?,
    };
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let contract = nexa_core::llm::model_contract::resolve_configured_model_contract(
        config.provider,
        config.base_url.as_deref(),
        &config.model,
    );
    let token = CancellationToken::new();
    let mut executor = AgentExecutor::new(Box::new(MeteredProvider { inner, calls: calls.clone() }), registry, AgentConfig {
        model: Some(config.model.clone()),
        system_prompt: format!("You are completing a repository task. Read relevant files, apply the requested change using the supplied file tools, and preserve unrelated behavior. The workspace is isolated. Do not invent test execution.\n\n{workspace_prompt}"),
        max_iterations: 32,
        max_tokens: Some(2048),
        native_vision: (!task.input_images.is_empty()).then_some(true),
        max_actual_tokens_per_run: Some(config.max_tokens_per_task),
        agent_timeout_secs: Some(config.timeout_seconds.min(u64::from(u32::MAX)) as u32),
        context_window_resolution: Some(contract.context_window(task.context_window_tokens.or_else(|| (mode == EvalMode::Scripted).then_some(128_000)))),
        catalog_limits_authoritative: Some(contract.catalog_authoritative),
        resolved_catalog_limits: contract.catalog_limits(),
        provider_type: Some(contract.provider_type),
            reasoning_enabled: config.reasoning_enabled,
        tool_approval_mode: if task.require_approval { ToolApprovalMode::Ask } else { ToolApprovalMode::AllowAll },
        trace_enabled: true,
        ..Default::default()
    }).with_cancel_token(token.clone()).with_skills_override(Vec::new()).with_auto_loaded_skills_override(Vec::new());
    if task.require_approval {
        executor = executor.with_approval_callback(Arc::new(|_| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(25)).await;
                ApprovalDecision::AllowOnce
            })
        }));
    }
    let db = Database::open_memory()?;
    let (tx, mut rx) = mpsc::channel(128);
    let events = tokio::spawn(async move {
        let (mut tools, mut failures, mut approvals, mut compactions) = (0, 0, 0, 0);
        let mut completed_tools = BTreeMap::<String, usize>::new();
        while let Some(event) = rx.recv().await {
            match event {
                AgentEvent::ToolRunCompleted { run } => {
                    tools += 1;
                    if run.status != nexa_core::agent::ToolRunStatus::Completed
                        || run.is_error == Some(true)
                    {
                        failures += 1;
                    } else {
                        *completed_tools.entry(run.tool_name).or_default() += 1;
                    }
                }
                AgentEvent::ApprovalRequested { .. } => approvals += 1,
                AgentEvent::AutoCompacted { .. } => compactions += 1,
                _ => {}
            }
        }
        (tools, failures, approvals, compactions, completed_tools)
    });
    let preparation_ms = task_started.elapsed().as_millis() as u64;
    let started = Instant::now();
    let mut input = vec![ContentPart::Text {
        text: task.prompt.clone(),
    }];
    input.extend(task.input_images.iter().map(|image| ContentPart::Image {
        media_type: image.mime_type.clone(),
        data: image.data.clone(),
    }));
    let run = executor.run(Vec::new(), input, &db, None, None, tx, 0);
    let result = tokio::time::timeout(Duration::from_secs(config.timeout_seconds), run).await;
    let executor_elapsed_ms = started.elapsed().as_millis() as u64;
    let (answer, error) = match result {
        Ok(Ok(message)) => (message.text_content(), None),
        Ok(Err(error)) => (String::new(), Some(error.to_string())),
        Err(_) => {
            token.cancel();
            (String::new(), Some("task deadline exceeded".into()))
        }
    };
    drop(executor);
    let (tool_calls, failed_tool_calls, approvals, compactions, completed_tools) = events.await?;
    let mut runtime_evidence_passed = (!task.require_approval || approvals > 0)
        && (!task.require_compaction || compactions > 0)
        && completed_tools.values().sum::<usize>() >= task.min_tool_calls
        && task
            .required_tool_counts
            .iter()
            .all(|(tool, count)| completed_tools.get(tool).copied().unwrap_or(0) >= *count)
        && task
            .required_tools
            .iter()
            .all(|tool| completed_tools.contains_key(tool));
    let oracle_started = Instant::now();
    let (oracle_passed, oracle_diagnostic) =
        check_oracle(task, outer.path(), &workspace_path, &answer).await?;
    let oracle_ms = oracle_started.elapsed().as_millis() as u64;
    let calls = calls.lock().unwrap().clone();
    let expected_images: Vec<_> = task
        .input_images
        .iter()
        .map(|image| provider::image_digest(&image.mime_type, &image.data))
        .collect();
    let observed_images = calls
        .first()
        .map(|call| call.input_image_digests.as_slice())
        .unwrap_or_default();
    runtime_evidence_passed &= observed_images == expected_images.as_slice();
    let observed_input_images = observed_images.len();
    let provider_invocation_ms = calls.iter().map(|call| call.elapsed_ms).sum();
    let mut outputs = Vec::new();
    for relative in &task.expected_files {
        let path = safe_relative(&workspace_path, relative)?;
        let bytes = std::fs::read(path).ok();
        outputs.push(FileEvidence {
            path: relative.clone(),
            digest: bytes
                .as_ref()
                .map(|bytes| blake3::hash(bytes).to_hex().to_string()),
            content: bytes
                .as_ref()
                .filter(|bytes| bytes.len() <= 32_768)
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned()),
        });
    }
    Ok(TaskReport {
        task_id: task.id.clone(),
        category: task.category.clone(),
        repetition,
        passed: error.is_none() && oracle_passed && runtime_evidence_passed,
        runtime_evidence_passed,
        compactions,
        completed_tools,
        input_images: task.input_images.len(),
        observed_input_images,
        context_window_tokens: task.context_window_tokens,
        oracle_passed,
        error,
        oracle_diagnostic,
        elapsed_ms: task_started.elapsed().as_millis() as u64,
        preparation_ms,
        executor_elapsed_ms,
        oracle_ms,
        provider_invocation_ms,
        orchestration_and_tools_ms: executor_elapsed_ms.saturating_sub(provider_invocation_ms),
        tool_calls,
        available_tools: tool_names,
        failed_tool_calls,
        approvals,
        reported_usage_cost: if mode == EvalMode::Live {
            config
                .prices
                .as_ref()
                .and_then(|prices| prices.estimate(&calls, contract.reasoning.key.api_style))
        } else {
            None
        },
        provider_invocations: calls,
        physical_attempt_count: None,
        cost_scope: "reported_provider_invocations",
        answer_digest: blake3::hash(answer.as_bytes()).to_hex().to_string(),
        outputs,
    })
}

pub async fn run_suite(
    mode: EvalMode,
    config: &EvalConfig,
    api_key: Option<&str>,
    filter: Option<&str>,
) -> EvalResult<SuiteReport> {
    if config.repetitions == 0
        || config.repetitions > 20
        || config.timeout_seconds == 0
        || config.timeout_seconds > 900
        || config.max_tokens_per_task == 0
    {
        return Err(
            "repetitions must be 1..20, timeout 1..900 seconds, and the token budget positive"
                .into(),
        );
    }
    if let Some(prices) = &config.prices {
        if [
            prices.input_per_million,
            prices.output_per_million,
            prices.cache_read_per_million,
            prices.cache_creation_per_million,
        ]
        .iter()
        .any(|n| !n.is_finite() || *n < 0.0)
            || prices.currency.trim().is_empty()
            || prices.source.trim().is_empty()
        {
            return Err(
                "token prices need finite nonnegative rates, a currency and a source".into(),
            );
        }
    }
    let node_version = tokio::process::Command::new("node")
        .arg("--version")
        .output()
        .await?;
    if !node_version.status.success() {
        return Err("Node.js is required for task oracles".into());
    }
    let source = provenance::verify_current_checkout()?;
    let (tasks, corpus_digest) = builtin_tasks()?;
    let tasks = tasks
        .into_iter()
        .filter(|task| filter.is_none_or(|filter| task.id.contains(filter)))
        .collect::<Vec<_>>();
    if tasks.is_empty() {
        return Err("No tasks match the filter".into());
    }
    let contract = nexa_core::llm::model_contract::resolve_configured_model_contract(
        config.provider,
        config.base_url.as_deref(),
        &config.model,
    );
    let mut cases = Vec::new();
    for repetition in 1..=config.repetitions {
        for task in &tasks {
            eprintln!("{} [{repetition}/{}]", task.id, config.repetitions);
            cases.push(run_task(task, repetition, mode, config, api_key).await?);
        }
    }
    let final_source = provenance::verify_current_checkout()?;
    if final_source.source_fingerprint != source.source_fingerprint {
        return Err("Source changed during evaluation; rerun from an immutable checkout".into());
    }
    Ok(SuiteReport {
        schema_version: 2,
        mode,
        score_kind: if mode == EvalMode::Live {
            "model_task_quality"
        } else {
            "scripted_runtime_contract"
        },
        source_sha: source.source_sha,
        source_dirty: source.source_dirty,
        source_fingerprint: source.source_fingerprint,
        compiled_source_sha: source.compiled_source_sha,
        compiled_source_dirty: source.compiled_source_dirty,
        compiled_source_fingerprint: source.compiled_source_fingerprint,
        corpus_digest,
        node_version: String::from_utf8_lossy(&node_version.stdout).trim().into(),
        target_triple: env!("NEXA_EVAL_BUILD_TARGET"),
        build_profile: env!("NEXA_EVAL_BUILD_PROFILE"),
        rustc_version: env!("NEXA_EVAL_RUSTC_VERSION"),
        model: config.model.clone(),
        provider: contract.provider_type,
        api_style: contract.reasoning.key.api_style,
        reasoning_enabled: config.reasoning_enabled,
        provider_endpoint_id: contract.reasoning.key.endpoint_id.clone(),
        repetitions: config.repetitions,
        max_tokens_per_task: config.max_tokens_per_task,
        max_output_tokens_per_request: 2048,
        timeout_seconds: config.timeout_seconds,
        prices: config.prices.clone(),
        passed: cases.iter().filter(|case| case.passed).count(),
        total: cases.len(),
        cases,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn costs_follow_adapter_input_counter_semantics() {
        use nexa_core::llm::reasoning_profile::ReasoningApiStyle;
        let prices = TokenPrices {
            input_per_million: 1.0,
            output_per_million: 2.0,
            cache_read_per_million: 0.1,
            cache_creation_per_million: 1.25,
            currency: "fixture".into(),
            source: "test rates".into(),
        };
        let calls = [ProviderInvocation {
            prompt_tokens: 100,
            completion_tokens: 20,
            cache_read_tokens: Some(80),
            cache_creation_tokens: Some(10),
            usage_reported: true,
            ..Default::default()
        }];
        assert!(
            (prices
                .estimate(&calls, ReasoningApiStyle::AnthropicMessages)
                .unwrap()
                - 160.5 / 1_000_000.0)
                .abs()
                < 1e-12
        );
        assert!(
            (prices
                .estimate(&calls, ReasoningApiStyle::OpenAiChatCompletions)
                .unwrap()
                - 70.5 / 1_000_000.0)
                .abs()
                < 1e-12
        );
        assert_eq!(
            prices.estimate(
                &[ProviderInvocation::default()],
                ReasoningApiStyle::AnthropicMessages
            ),
            None
        );
    }

    #[test]
    fn task_paths_cannot_escape_the_isolated_workspace() {
        for path in ["", "../oracle.mjs", "/tmp/file", "C:\\private\\file"] {
            assert!(
                safe_relative(Path::new("workspace"), path).is_err(),
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn oracle_rejects_early_successful_exit_and_assertion_replacement() {
        for candidate in [
            "process.exit(0);\n",
            "import assert from 'node:assert/strict'; assert.fail = () => {};\n",
            "import assert from 'node:assert'; assert.strict.fail = () => {};\n",
        ] {
            let mut task = builtin_tasks().unwrap().0.remove(0);
            task.files = BTreeMap::from([("candidate.mjs".into(), candidate.into())]);
            task.oracle = "import assert from 'node:assert/strict'; import {pathToFileURL} from 'node:url'; import path from 'node:path'; await import(pathToFileURL(path.join(process.argv[2],'candidate.mjs'))); assert.fail('invalid candidate must never pass');".into();
            let outer = tempfile::tempdir().unwrap();
            let workspace = outer.path().join("workspace");
            prepare_workspace(&task, &workspace).unwrap();
            let (passed, diagnostic) = check_oracle(&task, outer.path(), &workspace, "")
                .await
                .unwrap();
            assert!(!passed, "oracle bypass was accepted: {candidate}");
            assert!(
                diagnostic.contains("trusted completion point"),
                "{diagnostic}"
            );
        }
    }

    #[tokio::test]
    async fn oracle_accepts_completed_assertions_after_bounded_noisy_stdout() {
        let mut task = builtin_tasks().unwrap().0.remove(0);
        task.files = BTreeMap::from([(
            "candidate.mjs".into(),
            "process.stdout.write('noise'.repeat(100_000)); export const answer = 42;".into(),
        )]);
        task.oracle = "import assert from 'node:assert/strict'; import {pathToFileURL} from 'node:url'; import path from 'node:path'; const result = await import(pathToFileURL(path.join(process.argv[2],'candidate.mjs'))); assert.equal(result.answer,42);".into();
        let outer = tempfile::tempdir().unwrap();
        let workspace = outer.path().join("workspace");
        prepare_workspace(&task, &workspace).unwrap();
        let (passed, diagnostic) = check_oracle(&task, outer.path(), &workspace, "")
            .await
            .unwrap();
        assert!(passed, "valid completed oracle failed: {diagnostic}");
        assert!(diagnostic.is_empty());
    }

    #[tokio::test]
    async fn every_task_oracle_rejects_the_unmodified_workspace() {
        for task in builtin_tasks().unwrap().0 {
            let outer = tempfile::tempdir().unwrap();
            let workspace = outer.path().join("workspace");
            prepare_workspace(&task, &workspace).unwrap();
            let (passed, _) = check_oracle(&task, outer.path(), &workspace, "")
                .await
                .unwrap();
            assert!(
                !passed,
                "{} oracle must reject the initial task state",
                task.id
            );
        }
    }
}
