use std::collections::BTreeSet;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use ask_core::agent::{AgentConfig, AgentEvent, AgentExecutor};
use ask_core::db::Database;
use ask_core::error::CoreError;
use ask_core::llm::{create_provider, ContentPart, ProviderConfig, Usage};
use ask_core::search;
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

const DESCRIPTION: &str = "Spawn a short-lived subagent to handle an isolated subtask, gather an independent perspective, or critique another result. You can call this tool multiple times in parallel, pass it explicit evidence and acceptance criteria, narrow its source scope or tool access, and then synthesize or adjudicate the returned results yourself.";

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
    #[serde(default)]
    acceptance_criteria: Option<Vec<String>>,
    #[serde(default)]
    evidence_chunk_ids: Option<Vec<String>>,
    #[serde(default)]
    source_ids: Option<Vec<String>>,
    #[serde(default)]
    allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    parallel_group: Option<String>,
    #[serde(default)]
    deliverable_style: Option<String>,
    #[serde(default)]
    return_sections: Option<Vec<String>>,
}

#[derive(Default)]
struct EventCapture {
    usage_total: Usage,
    finish_reason: Option<String>,
    tool_events: Vec<serde_json::Value>,
    thinking: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct EvidenceHandoffItem {
    chunk_id: String,
    path: String,
    title: String,
    excerpt: String,
}

fn trim_optional(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn normalize_string_list(value: Option<Vec<String>>, limit: usize) -> Option<Vec<String>> {
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();

    for item in value.unwrap_or_default() {
        let trimmed = item.trim();
        if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
            continue;
        }
        normalized.push(trimmed.to_string());
        if normalized.len() >= limit {
            break;
        }
    }

    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn truncate_excerpt(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }

    let mut cut = max_chars;
    while cut > 0 && !content.is_char_boundary(cut) {
        cut -= 1;
    }
    let trimmed = content[..cut].trim_end();
    format!("{trimmed}...[truncated]")
}

fn build_subagent_system_prompt(base_prompt: &str, role: Option<&str>) -> String {
    let mut prompt = base_prompt.trim().to_string();
    prompt.push_str("\n\n## Subagent Instructions\n\n");
    prompt.push_str(
        "You are a short-lived worker spawned by another agent. Focus only on the delegated subtask. Keep your work scoped, use tools only when they materially help, and return a compact result for the supervisor agent rather than addressing the end user directly.",
    );
    prompt.push_str(
        "\n\nTreat supervisor-provided acceptance criteria as requirements. If explicit evidence handoff is provided, ground your answer in that evidence before doing broader retrieval. If you are one of several parallel workers, produce an independent result instead of speculating about what sibling workers might find.",
    );

    if let Some(role) = role.map(str::trim).filter(|value| !value.is_empty()) {
        prompt.push_str("\n\n## Assigned Role\n\n");
        prompt.push_str(role);
    }

    prompt
}

fn build_return_sections(args: &SpawnSubagentArgs) -> Vec<String> {
    normalize_string_list(args.return_sections.clone(), 8).unwrap_or_else(|| {
        vec![
            "Conclusion".to_string(),
            "Key evidence or reasoning".to_string(),
            "Risks or open questions".to_string(),
        ]
    })
}

fn resolve_source_scope(parent_scope: &[String], requested_scope: Option<&[String]>) -> Vec<String> {
    match requested_scope {
        Some(requested) if !requested.is_empty() => {
            if parent_scope.is_empty() {
                requested.to_vec()
            } else {
                let parent: BTreeSet<&str> = parent_scope.iter().map(String::as_str).collect();
                let narrowed: Vec<String> = requested
                    .iter()
                    .filter(|id| parent.contains(id.as_str()))
                    .cloned()
                    .collect();
                if narrowed.is_empty() {
                    parent_scope.to_vec()
                } else {
                    narrowed
                }
            }
        }
        _ => parent_scope.to_vec(),
    }
}

fn resolve_allowed_tools(base_allowed_tools: &[String], requested_allowed_tools: Option<&[String]>) -> Vec<String> {
    match requested_allowed_tools {
        Some(requested) if !requested.is_empty() => {
            let allowed: BTreeSet<&str> = base_allowed_tools.iter().map(String::as_str).collect();
            let narrowed: Vec<String> = requested
                .iter()
                .filter(|name| allowed.contains(name.as_str()))
                .cloned()
                .collect();
            if narrowed.is_empty() {
                base_allowed_tools.to_vec()
            } else {
                narrowed
            }
        }
        _ => base_allowed_tools.to_vec(),
    }
}

fn build_evidence_handoff(db: &Database, chunk_ids: Option<&[String]>) -> Vec<EvidenceHandoffItem> {
    chunk_ids
        .unwrap_or(&[])
        .iter()
        .take(8)
        .filter_map(|chunk_id| {
            let card = search::get_evidence_card(db, chunk_id).ok()?;
            Some(EvidenceHandoffItem {
                chunk_id: card.chunk_id.to_string(),
                path: card.document_path,
                title: card.document_title,
                excerpt: truncate_excerpt(&card.content, 1400),
            })
        })
        .collect()
}

fn build_subagent_request(
    args: &SpawnSubagentArgs,
    effective_source_scope: &[String],
    effective_allowed_tools: &[String],
    evidence_handoff: &[EvidenceHandoffItem],
) -> String {
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

    if let Some(group) = args
        .parallel_group
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request.push_str("\n\nParallel group:\n");
        request.push_str(group);
        request.push_str(
            "\nTreat this as an independent branch of work. Do not assume what sibling workers will conclude.",
        );
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

    if let Some(style) = args
        .deliverable_style
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request.push_str("\n\nDeliverable style:\n");
        request.push_str(style);
    }

    if let Some(criteria) = args.acceptance_criteria.as_ref().filter(|items| !items.is_empty()) {
        request.push_str("\n\nAcceptance criteria:\n");
        for item in criteria {
            request.push_str("- ");
            request.push_str(item);
            request.push('\n');
        }
    }

    if !effective_source_scope.is_empty() {
        request.push_str("\nSource scope restriction:\n");
        for source_id in effective_source_scope {
            request.push_str("- ");
            request.push_str(source_id);
            request.push('\n');
        }
    }

    if !effective_allowed_tools.is_empty() {
        request.push_str("\nDelegated tool access:\n");
        for tool_name in effective_allowed_tools {
            request.push_str("- ");
            request.push_str(tool_name);
            request.push('\n');
        }
    }

    if !evidence_handoff.is_empty() {
        request.push_str("\nEvidence handoff:\n");
        for evidence in evidence_handoff {
            request.push_str(&format!(
                "\n--- Evidence ---\n[chunk_id: {}]\nPath: {}\nTitle: {}\nExcerpt:\n{}\n",
                evidence.chunk_id, evidence.path, evidence.title, evidence.excerpt
            ));
        }
    }

    let sections = build_return_sections(args);
    request.push_str("\n\nReturn a concise result with these sections:\n");
    for (index, section) in sections.iter().enumerate() {
        request.push_str(&format!("{}. {}\n", index + 1, section));
    }

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
                "acceptance_criteria": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional checklist the subagent should satisfy before returning."
                },
                "evidence_chunk_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional chunk IDs that should be treated as handed-off evidence from the supervisor."
                },
                "source_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional narrower source scope for this subagent. When omitted, it inherits the supervisor scope."
                },
                "allowed_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional narrower tool whitelist for this subagent. Tool names must be from the delegated allowlist."
                },
                "parallel_group": {
                    "type": "string",
                    "description": "Optional label used when several subagents are exploring sibling branches in parallel."
                },
                "deliverable_style": {
                    "type": "string",
                    "description": "Optional style hint such as critique, plan, comparison, or verification report."
                },
                "return_sections": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional ordered section titles the subagent should use in its response."
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
        args.parallel_group = trim_optional(args.parallel_group);
        args.deliverable_style = trim_optional(args.deliverable_style);
        args.acceptance_criteria = normalize_string_list(args.acceptance_criteria.take(), 8);
        args.evidence_chunk_ids = normalize_string_list(args.evidence_chunk_ids.take(), 8);
        args.source_ids = normalize_string_list(args.source_ids.take(), 16);
        args.allowed_tools = normalize_string_list(args.allowed_tools.take(), 16);
        args.return_sections = normalize_string_list(args.return_sections.take(), 8);

        let provider = create_provider(self.provider_config.clone())
            .map_err(|e| CoreError::Llm(e.to_string()))?;

        let mut config = self.base_config.clone();
        config.max_iterations = args.max_iterations.unwrap_or(3).clamp(1, 6);
        config.max_tokens = Some(config.max_tokens.unwrap_or(2048).min(2048));
        config.system_prompt =
            build_subagent_system_prompt(&config.system_prompt, args.role.as_deref());

        let baseline_allowed_tools = normalize_allowed_tools(self.allowed_tools.as_deref());
        let effective_allowed_tools =
            resolve_allowed_tools(&baseline_allowed_tools, args.allowed_tools.as_deref());
        let effective_source_scope =
            resolve_source_scope(source_scope, args.source_ids.as_deref());
        let evidence_handoff = build_evidence_handoff(db, args.evidence_chunk_ids.as_deref());

        let tools = build_subagent_tool_registry(Some(&effective_allowed_tools));
        let executor = AgentExecutor::new(provider, tools, config);
        let request_text = build_subagent_request(
            &args,
            &effective_source_scope,
            &effective_allowed_tools,
            &evidence_handoff,
        );

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
                Some(effective_source_scope.clone()),
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
            "acceptanceCriteria": args.acceptance_criteria,
            "evidenceChunkIds": args.evidence_chunk_ids,
            "evidenceHandoff": evidence_handoff,
            "requestedSourceScope": args.source_ids,
            "effectiveSourceScope": effective_source_scope,
            "requestedAllowedTools": args.allowed_tools,
            "parallelGroup": args.parallel_group,
            "deliverableStyle": args.deliverable_style,
            "returnSections": args.return_sections,
            "result": result_text,
            "finishReason": capture.finish_reason,
            "usageTotal": capture.usage_total,
            "toolEvents": capture.tool_events,
            "thinking": if capture.thinking.is_empty() {
                None
            } else {
                Some(capture.thinking)
            },
            "sourceScopeApplied": !effective_source_scope.is_empty(),
            "allowedTools": effective_allowed_tools,
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
