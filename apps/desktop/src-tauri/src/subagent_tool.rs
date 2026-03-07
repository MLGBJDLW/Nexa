use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::mpsc;

use ask_core::agent::{AgentConfig, AgentEvent, AgentExecutor};
use ask_core::db::Database;
use ask_core::error::CoreError;
use ask_core::llm::{create_provider, ContentPart, ProviderConfig, Usage};
use ask_core::tools::{
    chunk_context_tool::ChunkContextTool,
    compare_tool::CompareTool,
    date_search_tool::DateSearchTool,
    document_info_tool::GetDocumentInfoTool,
    fetch_url_tool::FetchUrlTool,
    file_tool::FileTool,
    list_dir_tool::ListDirTool,
    list_documents_tool::ListDocumentsTool,
    list_sources_tool::ListSourcesTool,
    record_verification_tool::RecordVerificationTool,
    search_playbooks_tool::SearchPlaybooksTool,
    search_tool::SearchTool,
    statistics_tool::GetStatisticsTool,
    summarize_tool::{RetrieveEvidenceTool, SummarizeDocumentTool},
    update_plan_tool::UpdatePlanTool,
    Tool, ToolRegistry, ToolResult,
};

const DESCRIPTION: &str = "Spawn a short-lived subagent to handle an isolated subtask, gather an independent perspective, or critique another result. You can call this tool multiple times in parallel and then synthesize or compare the returned results yourself.";

type ToolFactory = fn() -> Box<dyn Tool>;

struct SubagentToolSpec {
    name: &'static str,
    enabled_by_default: bool,
    build: ToolFactory,
}

const SUBAGENT_TOOL_SPECS: &[SubagentToolSpec] = &[
    SubagentToolSpec {
        name: "search_knowledge_base",
        enabled_by_default: true,
        build: || Box::new(SearchTool),
    },
    SubagentToolSpec {
        name: "read_file",
        enabled_by_default: true,
        build: || Box::new(FileTool),
    },
    SubagentToolSpec {
        name: "retrieve_evidence",
        enabled_by_default: true,
        build: || Box::new(RetrieveEvidenceTool),
    },
    SubagentToolSpec {
        name: "list_sources",
        enabled_by_default: true,
        build: || Box::new(ListSourcesTool),
    },
    SubagentToolSpec {
        name: "list_documents",
        enabled_by_default: true,
        build: || Box::new(ListDocumentsTool),
    },
    SubagentToolSpec {
        name: "list_dir",
        enabled_by_default: true,
        build: || Box::new(ListDirTool),
    },
    SubagentToolSpec {
        name: "get_chunk_context",
        enabled_by_default: true,
        build: || Box::new(ChunkContextTool),
    },
    SubagentToolSpec {
        name: "fetch_url",
        enabled_by_default: true,
        build: || Box::new(FetchUrlTool),
    },
    SubagentToolSpec {
        name: "search_playbooks",
        enabled_by_default: true,
        build: || Box::new(SearchPlaybooksTool),
    },
    SubagentToolSpec {
        name: "get_document_info",
        enabled_by_default: true,
        build: || Box::new(GetDocumentInfoTool),
    },
    SubagentToolSpec {
        name: "compare",
        enabled_by_default: true,
        build: || Box::new(CompareTool),
    },
    SubagentToolSpec {
        name: "get_statistics",
        enabled_by_default: true,
        build: || Box::new(GetStatisticsTool),
    },
    SubagentToolSpec {
        name: "date_search",
        enabled_by_default: true,
        build: || Box::new(DateSearchTool),
    },
    SubagentToolSpec {
        name: "summarize_document",
        enabled_by_default: true,
        build: || Box::new(SummarizeDocumentTool),
    },
    SubagentToolSpec {
        name: "update_plan",
        enabled_by_default: true,
        build: || Box::new(UpdatePlanTool),
    },
    SubagentToolSpec {
        name: "record_verification",
        enabled_by_default: true,
        build: || Box::new(RecordVerificationTool),
    },
];

pub struct SubagentTool {
    provider_config: ProviderConfig,
    base_config: AgentConfig,
    allowed_tools: Option<Vec<String>>,
}

impl SubagentTool {
    pub fn new(
        provider_config: ProviderConfig,
        base_config: AgentConfig,
        allowed_tools: Option<Vec<String>>,
    ) -> Self {
        Self {
            provider_config,
            base_config,
            allowed_tools,
        }
    }
}

#[derive(Debug, Deserialize)]
struct SpawnSubagentArgs {
    task: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    expected_output: Option<String>,
    #[serde(default)]
    max_iterations: Option<u32>,
}

#[derive(Default)]
struct EventCapture {
    usage_total: Usage,
    finish_reason: Option<String>,
    tool_events: Vec<serde_json::Value>,
    thinking: Vec<String>,
}

fn trim_optional(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn build_subagent_system_prompt(base_prompt: &str, role: Option<&str>) -> String {
    let mut prompt = base_prompt.trim().to_string();
    prompt.push_str("\n\n## Subagent Instructions\n\n");
    prompt.push_str(
        "You are a short-lived worker spawned by another agent. Focus only on the delegated subtask. Keep your work scoped, use tools only when they materially help, and return a compact result for the supervisor agent rather than addressing the end user directly.",
    );

    if let Some(role) = role.map(str::trim).filter(|value| !value.is_empty()) {
        prompt.push_str("\n\n## Assigned Role\n\n");
        prompt.push_str(role);
    }

    prompt
}

fn build_subagent_request(args: &SpawnSubagentArgs) -> String {
    let mut request = String::from(
        "Complete the delegated task below. If information is missing, make the smallest reasonable assumption, state it briefly, and continue.\n\nTask:\n",
    );
    request.push_str(args.task.trim());

    if let Some(role) = args
        .role
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request.push_str("\n\nRequested perspective:\n");
        request.push_str(role);
    }

    if let Some(context) = args
        .context
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request.push_str("\n\nSupervisor context:\n");
        request.push_str(context);
    }

    if let Some(expected_output) = args
        .expected_output
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request.push_str("\n\nDesired output:\n");
        request.push_str(expected_output);
    }

    request.push_str(
        "\n\nReturn a concise result with these sections:\n1. Conclusion\n2. Key evidence or reasoning\n3. Risks or open questions",
    );

    request
}

fn default_subagent_tool_names() -> Vec<String> {
    SUBAGENT_TOOL_SPECS
        .iter()
        .filter(|spec| spec.enabled_by_default)
        .map(|spec| spec.name.to_string())
        .collect()
}

fn normalize_allowed_tools(allowed_tools: Option<&[String]>) -> Vec<String> {
    match allowed_tools {
        Some(names) => names
            .iter()
            .filter_map(|name| {
                let trimmed = name.trim();
                SUBAGENT_TOOL_SPECS
                    .iter()
                    .find(|spec| spec.name == trimmed)
                    .map(|_| trimmed.to_string())
            })
            .collect(),
        None => default_subagent_tool_names(),
    }
}

fn build_subagent_tool_registry(allowed_tools: Option<&[String]>) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    let allowed = normalize_allowed_tools(allowed_tools);
    for tool_name in allowed {
        if let Some(spec) = SUBAGENT_TOOL_SPECS
            .iter()
            .find(|spec| spec.name == tool_name)
        {
            registry.register((spec.build)());
        }
    }
    registry
}

#[async_trait]
impl Tool for SubagentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The concrete subtask for the delegated agent to complete."
                },
                "role": {
                    "type": "string",
                    "description": "Optional perspective or specialization, for example researcher, critic, planner, or implementer."
                },
                "context": {
                    "type": "string",
                    "description": "Optional context from the supervisor, such as another agent's draft, constraints, or evidence to critique."
                },
                "expected_output": {
                    "type": "string",
                    "description": "Optional description of the format or deliverable you want back."
                },
                "max_iterations": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 6,
                    "description": "Optional round budget for the subagent. Keep this small."
                }
            },
            "required": ["task"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        call_id: &str,
        arguments: &str,
        db: &Database,
        source_scope: &[String],
    ) -> Result<ToolResult, CoreError> {
        let mut args: SpawnSubagentArgs = serde_json::from_str(arguments).map_err(|e| {
            CoreError::InvalidInput(format!("Invalid spawn_subagent arguments: {e}"))
        })?;

        args.task = args.task.trim().to_string();
        if args.task.is_empty() {
            return Err(CoreError::InvalidInput(
                "spawn_subagent requires a non-empty task".into(),
            ));
        }

        args.role = trim_optional(args.role);
        args.context = trim_optional(args.context);
        args.expected_output = trim_optional(args.expected_output);

        let provider = create_provider(self.provider_config.clone())
            .map_err(|e| CoreError::Llm(e.to_string()))?;

        let mut config = self.base_config.clone();
        config.max_iterations = args.max_iterations.unwrap_or(3).clamp(1, 6);
        config.max_tokens = Some(config.max_tokens.unwrap_or(2048).min(2048));
        config.system_prompt =
            build_subagent_system_prompt(&config.system_prompt, args.role.as_deref());

        let allowed_tools = normalize_allowed_tools(self.allowed_tools.as_deref());
        let tools = build_subagent_tool_registry(Some(&allowed_tools));
        let executor = AgentExecutor::new(provider, tools, config);
        let request_text = build_subagent_request(&args);

        let (tx, mut rx) = mpsc::channel::<AgentEvent>(64);
        let event_task = tokio::spawn(async move {
            let mut capture = EventCapture::default();

            while let Some(event) = rx.recv().await {
                match event {
                    AgentEvent::ToolCallStart {
                        call_id,
                        tool_name,
                        arguments,
                    } => capture.tool_events.push(serde_json::json!({
                        "phase": "start",
                        "callId": call_id,
                        "toolName": tool_name,
                        "arguments": arguments,
                    })),
                    AgentEvent::ToolCallResult {
                        call_id,
                        tool_name,
                        content,
                        is_error,
                        artifacts,
                    } => capture.tool_events.push(serde_json::json!({
                        "phase": "result",
                        "callId": call_id,
                        "toolName": tool_name,
                        "content": content,
                        "isError": is_error,
                        "artifacts": artifacts,
                    })),
                    AgentEvent::Thinking { content } => {
                        if !content.trim().is_empty() {
                            capture.thinking.push(content);
                        }
                    }
                    AgentEvent::UsageUpdate { usage_total, .. } => {
                        capture.usage_total = usage_total;
                    }
                    AgentEvent::Done {
                        usage_total,
                        finish_reason,
                        ..
                    } => {
                        capture.usage_total = usage_total;
                        capture.finish_reason = finish_reason;
                    }
                    AgentEvent::TextDelta { .. }
                    | AgentEvent::Error { .. }
                    | AgentEvent::AutoCompacted { .. } => {}
                }
            }

            capture
        });

        let final_message = executor
            .run_with_source_scope(
                Vec::new(),
                vec![ContentPart::Text { text: request_text }],
                db,
                None,
                Some(source_scope.to_vec()),
                tx,
                0,
            )
            .await?;

        let capture = event_task.await.unwrap_or_default();
        let result_text = final_message.text_content().trim().to_string();
        let result_text = if result_text.is_empty() {
            "(Subagent returned no text.)".to_string()
        } else {
            result_text
        };

        let artifact = serde_json::json!({
            "kind": "subagent_result",
            "task": args.task,
            "role": args.role,
            "expectedOutput": args.expected_output,
            "result": result_text,
            "finishReason": capture.finish_reason,
            "usageTotal": capture.usage_total,
            "toolEvents": capture.tool_events,
            "thinking": if capture.thinking.is_empty() {
                None
            } else {
                Some(capture.thinking)
            },
            "sourceScopeApplied": !source_scope.is_empty(),
            "allowedTools": allowed_tools,
        });

        let mut content = String::from("Subagent result");
        if let Some(role) = artifact["role"].as_str() {
            content.push_str(&format!(" ({role})"));
        }
        content.push_str(":\n");
        content.push_str(artifact["result"].as_str().unwrap_or_default());

        Ok(ToolResult {
            call_id: call_id.to_string(),
            content,
            is_error: false,
            artifacts: Some(artifact),
        })
    }
}
