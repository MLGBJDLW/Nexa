# Tool Reference

Nexa ships with built-in tools that the AI agent can call during conversations,
plus tools from enabled MCP connectors. Knowledge tools use source scope;
file tools apply the configured file-access mode. Network, shell, desktop, connector, and live-terminal
tools declare separate trust and approval boundaries; they are not described as
knowledge-base reads.

## Schema authority and discovery

The runtime registry and each tool's executable schema are authoritative for
parameters and availability. The focused descriptions below explain usage and
boundaries; use `tool_search` to inspect the enabled tools for the current run.
Feature flags, the desktop host, configuration, and permissions can restrict
what is callable.

The following index is generated from the core JSON definitions with root
`npm run docs:generate`; `npm run docs:check` detects drift. It is a source-schema
index, not a promise that every listed tool is enabled in every conversation.
Some host tools and delegation schemas are assembled in Rust; see
[core registration](../crates/core/src/tools/mod.rs),
[terminal integration](../apps/desktop/src-tauri/src/terminal_agent_tool.rs),
and [subscription execution](SUBSCRIPTION_AGENTS.md).

<!-- BEGIN GENERATED TOOL SCHEMAS -->

| Core schema | Purpose (abridged; follow the schema for full rules) |
| --- | --- |
| [`agent_harness_dry_run`](../crates/core/prompts/tools/agent_harness_dry_run.json) | Run a read-only readiness preview for the local agent harness |
| [`archive_output`](../crates/core/prompts/tools/archive_output.json) | Archive an agent response or generated content as a new document in the knowledge base |
| [`browser_evidence_capture`](../crates/core/prompts/tools/browser_evidence_capture.json) | Open and inspect a public or loopback local-development web page in a real browser |
| [`code_intelligence`](../crates/core/prompts/tools/code_intelligence.json) | Find source-scoped code symbols or textual references in registered local source directories |
| [`compare_documents`](../crates/core/prompts/tools/compare_documents.json) | Compare content between two documents or chunks, showing differences and similarities |
| [`compile_document`](../crates/core/prompts/tools/compile_document.json) | Check the compilation status of knowledge base documents |
| [`computer_control`](../crates/core/prompts/tools/computer_control.json) | Perform one approval-gated action against a fresh Windows observation |
| [`computer_observe`](../crates/core/prompts/tools/computer_observe.json) | Observe the local Windows desktop without changing it |
| [`create_file`](../crates/core/prompts/tools/create_file.json) | Create, overwrite, or incrementally append UTF-8 plain-text files at the specified path |
| [`desktop_automation`](../crates/core/prompts/tools/desktop_automation.json) | Open or reveal files inside registered source directories on the user's visible desktop |
| [`download_asset`](../crates/core/prompts/tools/download_asset.json) | Download a supported public image asset (JPEG, PNG, WebP, or GIF) into the workspace with SSRF, redirect-hop, content-type, size, and output-path validation |
| [`edit_file`](../crates/core/prompts/tools/edit_file.json) | Edit an existing plain-text file or create a new plain-text file |
| [`extract_image_text`](../crates/core/prompts/tools/extract_image_text.json) | Extract visible text from a local image using the app's PaddleOCR runtime |
| [`fetch_url`](../crates/core/prompts/tools/fetch_url.json) | Fetch and read the text content of a public web page with SSRF and redirect-hop validation |
| [`generate_image`](../crates/core/prompts/tools/generate_image.json) | Generate an image using the provider configured in Settings and return an in-chat preview artifact |
| [`get_chunk_context`](../crates/core/prompts/tools/get_chunk_context.json) | Retrieve a chunk and its surrounding parent/child context window from the same document, ordered by chunk_index |
| [`get_document_info`](../crates/core/prompts/tools/get_document_info.json) | Get detailed metadata about a specific document in the knowledge base by its path or document ID |
| [`get_goal`](../crates/core/prompts/tools/get_goal.json) | Read the durable execution goal for the current conversation, including its objective and lifecycle status. |
| [`get_related_concepts`](../crates/core/prompts/tools/get_related_concepts.json) | Explore the knowledge base at a high level: browse the wiki index, generate a Map of Content for a topic, find hot concepts, get exploration suggestions, view query trends, or i... |
| [`get_statistics`](../crates/core/prompts/tools/get_statistics.json) | Get knowledge base statistics including total sources, documents, chunks, storage size, and last indexed time |
| [`glob_files`](../crates/core/prompts/tools/glob_files.json) | Find files and directories by glob pattern inside registered source directories using a safe ripgrep-style traversal that respects source scope, hidden-file settings, and gitign... |
| [`grep_files`](../crates/core/prompts/tools/grep_files.json) | Alias of search_files using familiar grep/rg terminology |
| [`judge_subagent_results`](../crates/core/prompts/tools/judge_subagent_results.json) | Adjudicate or rank multiple delegated worker results using a structured rubric |
| [`list_dir`](../crates/core/prompts/tools/list_dir.json) | List contents of a directory |
| [`list_documents`](../crates/core/prompts/tools/list_documents.json) | List documents in a specific knowledge-base source |
| [`list_sources`](../crates/core/prompts/tools/list_sources.json) | List all registered knowledge-base sources |
| [`manage_agent_memory`](../crates/core/prompts/tools/manage_agent_memory.json) | Record, search, list, or delete local agent procedural memories |
| [`manage_persona`](../crates/core/prompts/tools/manage_persona.json) | List available personas, inspect the current conversation persona, or switch the active conversation persona for future turns |
| [`manage_playbook`](../crates/core/prompts/tools/manage_playbook.json) | Create, update, list, get details of, add citations to, or delete a playbook |
| [`manage_project_memory`](../crates/core/prompts/tools/manage_project_memory.json) | List, search, record, update, archive, or delete memories for the active Project |
| [`manage_skill`](../crates/core/prompts/tools/manage_skill.json) | List, load, activate, inspect available skills and their bundled resources, execute a declared script resource helper through the skill resource helper sandbox, and create, insp... |
| [`manage_source`](../crates/core/prompts/tools/manage_source.json) | Add, remove, or refresh knowledge source directories |
| [`manage_user_memory`](../crates/core/prompts/tools/manage_user_memory.json) | List, search, record, update, or delete cross-session user memories |
| [`multi_edit`](../crates/core/prompts/tools/multi_edit.json) | Apply multiple exact text replacements to one existing plain-text file in a single atomic operation |
| [`office_artifact`](../crates/core/prompts/tools/office_artifact.json) | Inspect, assess, create, edit, verify, publish, discard, or restore DOCX, XLSX, and PPTX artifacts through Nexa's transactional OfficeArtifactEngine |
| [`open_in_nexa`](../crates/core/prompts/tools/open_in_nexa.json) | Open an authorized local file inside Nexa |
| [`prepare_document_tools`](../crates/core/prompts/tools/prepare_document_tools.json) | Check or prepare the local Python-backed document tools used by the Office skills |
| [`project_tool`](../crates/core/prompts/tools/project_tool.json) | Discover, describe, and run source-scoped project-local tool manifests |
| [`query_knowledge_graph`](../crates/core/prompts/tools/query_knowledge_graph.json) | Query the compiled entity relationship graph as a compact navigation index before retrieving full evidence |
| [`read_file`](../crates/core/prompts/tools/read_file.json) | Read a file by path |
| [`read_files`](../crates/core/prompts/tools/read_files.json) | Read multiple files in a single call |
| [`record_verification`](../crates/core/prompts/tools/record_verification.json) | Record what was verified before finishing a multi-step task |
| [`reindex_document`](../crates/core/prompts/tools/reindex_document.json) | Trigger re-indexing of a specific document by path or an entire source directory |
| [`request_user_input`](../crates/core/prompts/tools/request_user_input.json) | Ask the user one to six concise, structured questions when their input is genuinely needed |
| [`retrieve_evidence`](../crates/core/prompts/tools/retrieve_evidence.json) | Retrieve specific evidence chunks by their chunk IDs |
| [`run_health_check`](../crates/core/prompts/tools/run_health_check.json) | Run knowledge base health diagnostics to find stale documents, orphaned content, duplicate entities, and coverage gaps |
| [`search_by_date`](../crates/core/prompts/tools/search_by_date.json) | Browse documents by modification/creation date range |
| [`search_files`](../crates/core/prompts/tools/search_files.json) | Search plain-text files by content inside registered source directories, similar to a safe rg/ripgrep query |
| [`search_knowledge_base`](../crates/core/prompts/tools/search_knowledge_base.json) | Search the local knowledge base using graph-guided hybrid retrieval: entity/document graph planning, full-text search, vector search, graph expansion, and reranking |
| [`search_playbooks`](../crates/core/prompts/tools/search_playbooks.json) | Search existing playbooks by topic or keyword |
| [`search_sessions`](../crates/core/prompts/tools/search_sessions.json) | Search prior conversation messages across local sessions |
| [`spawn_subagent_batch`](../crates/core/prompts/tools/spawn_subagent_batch.json) | Spawn a batch of short-lived subagents for parallel fan-out research, critique, comparison, or templated workflows |
| [`spawn_subagent`](../crates/core/prompts/tools/spawn_subagent.json) | Start a short-lived subagent and immediately return its stable agent id |
| [`submit_feedback`](../crates/core/prompts/tools/submit_feedback.json) | Submit feedback (upvote, downvote, or pin) on a search result chunk to improve future search relevance |
| [`summarize_document`](../crates/core/prompts/tools/summarize_document.json) | Retrieve all indexed chunks of a document in order, suitable for full-document summarization |
| [`synthesize_speech`](../crates/core/prompts/tools/synthesize_speech.json) | Turn text into a spoken-audio preview using the cloud TTS provider configured in Settings |
| [`tool_search`](../crates/core/prompts/tools/tool_search.json) | Search the enabled built-in and MCP tool catalog by name and description |
| [`update_goal`](../crates/core/prompts/tools/update_goal.json) | Update the durable execution goal for the current conversation |
| [`update_plan`](../crates/core/prompts/tools/update_plan.json) | Create or update a short execution plan for the current task |
| [`update_scratchpad`](../crates/core/prompts/tools/update_scratchpad.json) | Update the per-conversation agent scratchpad — a small self-maintained notebook visible at the start of every turn via the system prompt |
| [`web_research_context`](../crates/core/prompts/tools/web_research_context.json) | Build a compact model-ready web research context pack |
| [`web_search`](../crates/core/prompts/tools/web_search.json) | Search the public web through Nexa's built-in no-key providers |
| [`write_note`](../crates/core/prompts/tools/write_note.json) | Create or update a note file in the knowledge base |

<!-- END GENERATED TOOL SCHEMAS -->

---

## 🔍 Search & Retrieval

### `tool_search`

Search the enabled built-in and MCP tool catalog by name and description. `tool_search` is the resident discovery lane for dynamic tool visibility: when a needed enabled tool is hidden from the current model step, matching results activate that tool for the next step. Disabled MCP connectors are not discoverable until connected.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `query` | string | yes | Natural language query or tool-name fragment |
| `limit` | integer | no | Max matches, 1-20 (default 8) |

> **Example:** Ask which enabled tool should handle source-scoped text search, document comparison, or an enabled connector capability.

---

### `search_knowledge_base`

Hybrid full-text (BM25) and vector search across all indexed content. Returns evidence cards with content, source paths, relevance scores, chunk IDs for citation, and trust metadata. Supports batch queries via the `queries` parameter for synonym/variant expansion in a single call.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `query` | string | no* | Concise noun-phrase search query |
| `queries` | string[] | no* | Multiple queries merged via rank fusion (overrides `query`) |
| `limit` | integer | no | Max results, 1–20 (default 5) |
| `source_ids` | string[] | no | Restrict to specific source IDs |
| `file_types` | string[] | no | Filter by type: `markdown`, `plaintext`, `log`, `pdf`, `docx`, `excel`, `pptx` |
| `date_from` | string | no | ISO 8601 lower bound on modification date |
| `date_to` | string | no | ISO 8601 upper bound on modification date |

> **Example:** Find notes about OAuth implementation from the last month using multiple keyword variants in one call.

`*` Provide either `query` or a non-empty `queries` array. Use at most two
meaningfully different variants. If one or two attempts remain weak, inspect
files or directories instead of repeating small query variations.

Artifact contract:

- `kind: "searchResults"`
- `evidenceCards`: citation-ready evidence cards
- `search`: query, result count, timing, mode, and query count
- `trustBoundary`: local-source evidence, read-only, cannot instruct
- `contract`: source role and authority notes for the model

Graph-guided planning, candidate expansion, reranking, and context packing also
report `graphRetrieval`, `retrievalConfidence`, `ragStrategy`, and `contextPack`.
Use these as retrieval diagnostics; supporting summaries and graph signals do
not replace direct chunks for detailed factual claims. Indexed web evidence can
also carry its explicit source URL. See
[the full schema](../crates/core/prompts/tools/search_knowledge_base.json).

Validation failures return `kind: "toolContractError"` artifacts with `code`, `message`, `expectedFormat`, `retryable`, and `trustBoundary`, so the model can correct the call instead of surfacing a raw schema error.

---

### `retrieve_evidence`

Retrieve original chunk text by ID for precise citation. Returns raw content together with source path and document title.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `chunk_ids` | string[] | yes | List of chunk UUIDs to retrieve |

> **Example:** Fetch the exact text of a search result to quote it accurately with `[cite:CHUNK_ID]`.

---

### `query_knowledge_graph`

Navigate the compiled graph before fetching supporting documents. `action` is
`context`, `map` (context alias), `related`, `path`, or `search`. Entity/path
actions use `entity_name` and, for a path, `target_name`. Source, path, entity,
relationship, and strength filters remain constrained by the active source scope.

Use graph output to select evidence worth retrieving. A relationship, inferred
path, or co-occurrence is not a citation by itself. See the
[complete schema](../crates/core/prompts/tools/query_knowledge_graph.json) and
[knowledge guide](KNOWLEDGE_AND_RETRIEVAL.md).

---

### `get_chunk_context`

Get surrounding chunks from the same document for expanded context around a search result.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `chunk_id` | string | yes | UUID of the target chunk |
| `context_chunks` | integer | no | Chunks before/after to include (default 2, max 5) |

> **Example:** A search hit looks relevant but incomplete — fetch the paragraphs before and after it.

---

### `search_playbooks`

Search playbook titles, descriptions, goals, and cited chunk content by keyword.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `query` | string | yes | Keywords or phrases to match |

> **Example:** Check if a playbook about "deployment checklist" already exists before creating a new one.

---

### `search_by_date`

Browse documents by modification/creation date range. Returns a chronological document list.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `after` | string | no | ISO 8601 date — documents modified after this point |
| `before` | string | no | ISO 8601 date — documents modified before this point |
| `source_id` | string | no | Filter to a specific source |
| `limit` | integer | no | Max documents, 1–200 (default 50) |
| `order` | string | no | `newest` or `oldest` (default `newest`) |

> **Example:** Find everything you worked on last week across all sources.

---

## 📖 Reading & Analysis

### File Tool Matrix

Use this quick routing guide when a request is about files or documents:

| Scenario | Preferred tool | File types / scope | Relative source-root path? | Notes |
|-----------|----------------|--------------------|----------------------------|-------|
| Locate a file or browse a folder | `list_dir` | Any file/folder inside a source | yes | Best first step when the exact path is unknown or ambiguous |
| Locate files by glob | `glob_files` | Any source-scoped file path | yes | Safe ripgrep-style traversal; respects hidden settings and gitignore files |
| Search inside local files by text or regex | `search_files` / `grep_files` | Plain-text files inside a source | yes | Safe rg-style search with line numbers; use after/beside KB search when exact file locations matter |
| Find code symbols or references | `code_intelligence` | Code/text files inside a source | yes | Lightweight declaration/reference lookup before broad reads; use for functions, types, components, commands, and domain terms |
| Discover or run repo-defined workflows | `project_tool` | `.nexa/tools/*.json`, `.agents/tools/*.json` | yes | Project-local manifest API for repeatable lint/test/codegen/diagnostic commands; `run` requires approval |
| Read a named file | `read_file` | Text, PDF, DOCX, XLSX, PPTX, image text extraction | yes | Supports line windows via `start_line` and `max_lines` |
| Inspect document metadata or index state | `get_document_info` | Indexed documents | yes | Good for source ID, chunk count, MIME type, citation info |
| Compare two files or indexed chunks | `compare_documents` | Text or parsed document content | yes for file paths | Use chunk IDs when you already know the exact evidence |
| Create a new plain-text file | `create_file` | Text-based files only | yes | For new `.md`, `.txt`, `.json`, `.rs`, etc. |
| Edit an existing plain-text file | `edit_file` | Text-based files only | yes | Exact `str_replace` only; must match once |
| Apply several coordinated text edits | `multi_edit` | Text-based files only | yes | Atomic multi-replacement with one checkpoint; all edits succeed or no file changes |
| Create, edit, verify, publish, or restore an Office file | `office_artifact` | DOCX, XLSX, PPTX | yes | Typed guarantees, candidate gating, validation/evidence, receipts, and hash-guarded restore |
| Edit/convert/render PDF or use an Office escape hatch | `run_shell` + `doc-script-editor` | DOCX, XLSX, PPTX, PDF | yes | Compatibility operations, extraction, conversion, rendering, and low-level OOXML edits |
| Compatibility fallback for very simple new Office files | `generate_docx`/`generate_xlsx`/`ppt_generate` | DOCX, XLSX, PPTX | yes | Use only when Python is unavailable or the schema fully covers the request |
| Refresh indexed content after file changes | `reindex_document` | File path or whole source | yes for file path | Use when external edits are not reflected in search/results yet |

Path guidance:
Use source-root relative paths like `notes/today.md` when the file clearly belongs to one registered source.
Use absolute paths when the user already supplied one or when a relative path could match multiple sources.

For the general read/edit/create tools, restricted mode uses the active source
scope, non-restricted registered-source modes can use all registered sources,
and open mode can accept absolute local paths outside sources. The actual tool
policy remains authoritative: `desktop_automation`, for example, still requires
a registered source even when a general file tool has broader access.

### Tool Authoring Quality Bar

When adding or changing tools, optimize for model-call correctness rather than developer convenience:

- Name parameters exactly and consistently; avoid aliases unless the tool explicitly supports them.
- Make required fields match runtime validation. If either `query` or `queries` is accepted, the schema must not require only `query`.
- Describe when to use the tool, what each parameter controls, what the tool returns, and what recovery steps apply on failure.
- Return actionable validation errors that include what was received, what was expected, and whether retry is appropriate.
- Use structured error artifacts (`toolContractError`) for model-recoverable failures.
- Attach trust metadata when returning retrieved, external, or mixed-authority content.
- Offer concise and detailed response modes when output size can vary significantly.
- Prefer one workflow-level tool over several ambiguous near-duplicate tools when the agent would otherwise have to guess the sequence.
- Every registered tool must expose a `ToolCapabilityDescriptor` through the registry. Treat it as the Nexa capability package manifest for the invocation: ecosystem surface, UI render kind, runtime scheduling capabilities, resource keys, access category, read/write/execute/network capability, approval need, risk level, and risk reason. Settings, approval UI, scheduling, and stream projection should read this descriptor instead of maintaining separate name-based tables.
- Object-shaped tool schemas automatically include `wait_for_previous`. The model can set it to `true` when a tool call depends on files, artifacts, or command output from an earlier tool call in the same turn; the scheduler will start a new execution batch before that call.
- Approval policy is target-aware. Shell commands are keyed by command prefix, file tools by resolved file resource, network tools by host, and MCP tools by server/tool identity. Use the target-aware policy APIs for new approval flows; legacy per-tool policies remain as a fallback only.
- Provider argument aliases, scalar types, and enum casing are canonicalized through one registry algorithm before scheduling, capability classification, policy evaluation, approval display, and execution. A tool must never execute a value that the approval path interpreted differently.
- Tool results can expose separate output channels through `ToolOutput`: `llm_content` for the next model call, `display_content` for the UI, `data` for structured payloads, `artifacts` for auxiliary JSON, and `attachments` for rich outputs. Existing `ToolResult.content` remains the display fallback for older tools.

### `read_file`

Read a file with an optional line range under the configured file-access mode.
Paths may be absolute or relative to a source root; open mode also permits
absolute local paths outside registered sources. In addition to plain text,
the tool can extract readable text from PDF, DOCX, XLSX, PPTX, and images when
the corresponding runtime is available.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | yes | Absolute path or path relative to a source root |
| `start_line` | integer | no | 1-based start line (default 1) |
| `max_lines` | integer | no | Max lines to return (default 100) |

> **Example:** Read lines 50–80 of a long configuration file to inspect a specific section.

---

### `list_sources`

List all registered knowledge-base source directories. Returns each source's ID, root path, document count, and last scan time. Takes no parameters.

> **Example:** Discover available source IDs to scope a search to a specific folder.

---

### `list_documents`

List documents in a specific source with pagination. Returns file path, title, MIME type, size, and last modified date.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `source_id` | string | yes | Source ID (from `list_sources`) |
| `limit` | integer | no | Max documents, 1–200 (default 50) |
| `offset` | integer | no | Pagination offset (default 0) |

> **Example:** Browse the first 20 documents in your "notes" source to find a specific file.

---

### `list_dir`

Browse directory structure with optional recursion and glob filtering.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | yes | Directory path (absolute or relative to a source root) |
| `recursive` | boolean | no | Recurse into subdirectories (default false) |
| `max_depth` | integer | no | Max recursion depth (default 3) |
| `pattern` | string | no | Filename glob filter (e.g. `*.md`, `*.pdf`) |

> **Example:** List all Markdown files recursively in a project folder.

---

### `glob_files`

Find source-scoped files and directories by glob pattern. Traversal respects source scope, hidden-file settings, and gitignore files.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `pattern` | string | no* | Glob pattern such as `*.md` or `**/README.*` |
| `patterns` | string[] | no* | Multiple glob patterns; overrides `pattern` |
| `path` | string | no | Directory to search; omitted means all current source-scope directories |
| `include_hidden` | boolean | no | Include dotfiles and hidden directories (default false) |
| `include_dirs` | boolean | no | Include matching directories as well as files (default false) |
| `max_results` | integer | no | Max paths, 1-500 (default 100) |

> **Example:** Find every Markdown note matching `notes/**/*.md` before selecting files to read.

---

### `search_files`

Search plain-text files by content inside registered source directories. This is a safe rg-style search tool for exact text, phrases, or regex patterns when line numbers and local file locations matter.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `query` | string | yes | Literal text or regex pattern to search for |
| `path` | string | no | File or directory path; omitted means all current source-scope directories |
| `regex` | boolean | no | Treat `query` as regex (default false) |
| `case_sensitive` | boolean | no | Use case-sensitive matching (default false) |
| `include_globs` | string[] | no | Include patterns such as `*.md` or `notes/**/*.txt` |
| `exclude_globs` | string[] | no | Exclude patterns such as `**/archive/**` |
| `max_results` | integer | no | Max matching lines, 1-200 (default 50) |
| `context_lines` | integer | no | Surrounding lines before/after each match, 0-3 (default 0) |
| `include_hidden` | boolean | no | Include dotfiles and hidden directories (default false) |

> **Example:** Find every source-scoped Markdown line mentioning a project name before editing the relevant note.

`grep_files` is an alias with the same parameters for users and prompts that naturally ask to grep or rg local files.

---

### `code_intelligence`

Find declaration-like code symbols or textual references inside registered source directories. This is a source-scoped local scanner for code navigation, not a full language server.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `action` | string | yes | `symbols` for declaration-like definitions, `references` for matching source lines |
| `query` | string | yes | Symbol name, identifier, fragment, or term |
| `path` | string | no | File or directory path; omitted means all current source-scope directories |
| `max_results` | integer | no | Max matches, 1-300 (default 80) |
| `case_sensitive` | boolean | no | Use case-sensitive matching (default false) |
| `whole_word` | boolean | no | For references, match identifier-like queries as whole words (default true) |
| `include_hidden` | boolean | no | Include dotfiles and hidden directories (default false) |

Returns `kind: "codeIntelligenceResults"` with searched file counts, truncation state, and matches containing `path`, `lineNumber`, `kind`, optional `name`, and `preview`. Use `symbols` first when you need likely definitions, then `references` to estimate call sites or usage.

---

### `project_tool`

Discover, describe, and run project-local tools declared by source-scoped manifests. Manifests live at `.nexa/tools/*.json` or `.agents/tools/*.json` under a registered source root. `list` and `describe` are read-only; `run` executes the manifest command without shell interpolation and requires approval.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `action` | string | yes | `list`, `describe`, or `run` |
| `name` | string | no* | Manifest tool name for `describe` or `run` |
| `manifestHash` | string | no* | Current manifest hash returned by `list` or `describe`; required for `run` |
| `arguments` | object | no | Scalar values used to expand command arg placeholders like `{{path}}` |

\* `name` is required for `describe` and `run`; `manifestHash` is required for `run`.

Manifest shape:

```json
{
  "name": "lint",
  "description": "Run the project lint check",
  "parameters": {
    "type": "object",
    "properties": {
      "path": { "type": "string" }
    }
  },
  "command": {
    "program": "npm",
    "args": ["run", "lint", "--", "{{path}}"],
    "cwd": ".",
    "timeoutSecs": 120
  },
  "access": {
    "read": true,
    "write": false,
    "execute": true,
    "network": false
  }
}
```

Command execution uses argv directly, not a shell. `program` must be a program name, `cwd` must stay inside the source root, `timeoutSecs` must be between 1 and 1800, and placeholder values must be JSON scalars.

Approval memory for `project_tool run` is keyed by manifest name plus the short manifest hash, so allowing `lint` for the session does not allow `test`, `deploy`, any other project-local tool, or a later edited `lint` manifest. The Settings → Extensions → Project tools panel shows the manifest path, command preview, declared access, validation errors, and the hash a run must use.

---

### `get_document_info`

Get detailed metadata about a single document — file path, size, modification time, chunk count, indexing status, and source information.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | no* | Document path (absolute or relative to a source root) |
| `document_id` | string | no* | UUID of the document |

\* At least one of `path` or `document_id` must be provided.

> **Example:** Check how many chunks a large PDF was split into and when it was last indexed.

---

### `open_in_nexa`

Open an authorized local artifact in Nexa's preview panel or Browser Workspace.
The required `path` may be absolute or relative to an authorized source root;
optional `line` selects a one-based text line. For HTML, `assets` lists at most
256 local dependencies under the HTML parent, each checked independently.

The receipt confirms that the host opened the preview. It does not verify the
document's contents or visual quality. Unsupported preview types return an
error instead of launching an external application. See
[Local HTML preview](local-html-preview.md) and the
[schema](../crates/core/prompts/tools/open_in_nexa.json).

---

### `compare_documents`

Compare content between two documents or chunks, showing differences and similarities. Accepts file paths or chunk IDs.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path_a` | string | no | First document path (absolute or relative to a source root) |
| `path_b` | string | no | Second document path (absolute or relative to a source root) |
| `chunk_id_a` | string | no | UUID of the first chunk (alternative to `path_a`) |
| `chunk_id_b` | string | no | UUID of the second chunk (alternative to `path_b`) |

Provide either both paths or both chunk IDs.

> **Example:** Cross-reference two versions of a design document to find what changed.

---

### `summarize_document`

Retrieve all indexed chunks of a document in order, suitable for full-document summarization.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | no* | File path of the document |
| `document_id` | string | no* | UUID of the document |
| `max_chunks` | integer | no | Max chunks to return (default 100) |

\* At least one of `path` or `document_id` must be provided.

> **Example:** Pull the full indexed content of a 30-page report so the agent can summarize it.

---

### `get_statistics`

Knowledge base health metrics — total sources, documents, chunks, storage size, and last indexed time.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `source_id` | string | no | Filter stats to a specific source |

> **Example:** Check the overall size and freshness of your indexed knowledge base.

---

## ✏️ Writing & Editing

### `write_note`

Create, append to, or overwrite note files (.md, .txt, .org, .rst) in a source's `notes/` subdirectory. Ideal for saving research syntheses, meeting summaries, or curated findings.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `filename` | string | yes | Note filename (e.g. `meeting-summary.md`) |
| `content` | string | yes | Markdown-formatted text content |
| `mode` | string | no | `create` (default), `append`, or `overwrite` |
| `source_id` | string | no | Target source directory (defaults to first available) |

> **Example:** Save a multi-source research synthesis as a new Markdown note for future reference.

---

### `edit_file`

Edit existing plain-text files via string replacement or create new plain-text files within registered source directories. Paths may be absolute or relative to a source root.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | yes | File path (absolute or relative to a source root) |
| `action` | string | no | `str_replace`, `replace` (alias), or `create`; inferred from the supplied fields when omitted |
| `old_str` | string | no | Exact text to find (for `str_replace`; must match once) |
| `new_str` | string | no | Replacement text (for `str_replace`) or file content (for `create`) |
| `start_line` | integer | no | One-based inclusive start of the replacement search range |
| `end_line` | integer | no | One-based inclusive end of the replacement search range |

Use `office_artifact` for DOCX/XLSX/PPTX candidate-based creation and edits.
Use `run_shell` + `doc-script-editor` for PDF, conversion/rendering, or OOXML
compatibility work. `edit_file` handles ordinary text, not Office/PDF binaries.
The full schema also documents `old_string`, `new_string`, and `content` aliases.

`str_replace` operates on UTF-8 char boundaries, so replacements containing multi-byte characters (CJK text, emoji, etc.) are handled safely without byte-slice panics.

> **Example:** Fix a typo in an existing text document or create a new configuration file.

---

### `multi_edit`

Apply multiple exact text replacements to one existing plain-text file in a single atomic operation. The tool validates each edit in order before writing; if any edit is missing or ambiguous, no file is changed. A restorable file checkpoint is created before the write.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | yes | File path (absolute or relative to a source root) |
| `edits` | object[] | yes | Ordered replacements, max 20 |
| `edits[].old_str` | string | yes | Exact text to find; must match once unless `replace_all` is true |
| `edits[].new_str` | string | no | Replacement text; omitted means delete the old text |
| `edits[].replace_all` | boolean | no | Replace every occurrence for that edit (default false) |
| `edits[].start_line` | integer | no | Optional 1-based inclusive line range start |
| `edits[].end_line` | integer | no | Optional 1-based inclusive line range end |

Use `office_artifact` for Office candidates and `run_shell` + `doc-script-editor`
for PDF or compatibility operations. `multi_edit` is for plain-text changes.

> **Example:** Update three related headings in a Markdown note with one checkpointed operation.

---

### `create_file`

Create, overwrite, or append a UTF-8 text file under the configured file-access
mode. Parent directories are created for create/overwrite operations.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | yes | Output file path (absolute or relative to a source root) |
| `content` | string | yes | Plain-text content to write |
| `mode` | string | no | `create` (default), `overwrite`, or `append` |
| `expected_bytes` | integer | for append | Exact current UTF-8 byte length from the previous successful result |
| `overwrite` | boolean | no | Deprecated alias for overwrite mode; do not combine with another explicit mode |

For long text, create the first chunk and append ordered chunks using the
returned `writeProgress.nextExpectedBytes`. Set `wait_for_previous` for
dependent calls within one assistant turn. If the byte precondition fails,
inspect current state before retrying. See
[Live file-tool streaming](LIVE_FILE_TOOL_STREAMING.md#resumable-plain-text-writes).

Do not use `create_file` for DOCX/XLSX/PPTX/PDF. Use `office_artifact` for DOCX/XLSX/PPTX work and `run_shell` + `doc-script-editor` for PDF or compatibility escape hatches. The format-specific generators are fallbacks for very simple new files only.

> **Example:** Create a new Markdown draft under `notes/` or add a config file in a nested folder.

---

### `office_artifact`

The preferred DOCX/XLSX/PPTX lifecycle is `capabilities`/`assess` → `execute` → `decide` → optional `restore`. `execute` creates a validated candidate by default and does not touch the destination. `decide: publish` atomically publishes it and returns a receipt; `restore` refuses to overwrite a destination that changed after publication.

Calls require `action` and `workspace_root`; action-specific request shapes and
live-host operations are defined in the
[closed schema](../crates/core/prompts/tools/office_artifact.json). The
[Office add-in guide](../integrations/office-addin/README.md) covers its separate
pairing and deployment path.

Requests use `requestVersion: 2`, a format and intent, typed operations, optional `preconditions.sourceSha256`, and explicit guarantees (`quality`, `preservation`, `calculation`, `render`). Inline validation requires `contractVersion: 2`; all schema fields are closed and unknown fields fail. `quality: publish` requires candidate-SHA-bound rendered evidence. `quality: native` and XLSX `calculation: native` require Microsoft Office COM. LibreOffice recalculation is labeled `compatible`, never Excel-native.

The adapter contract reports local Open XML, LibreOffice-compatible, Windows COM, and disconnected Office.js-live surfaces separately. A local `.nexa/office-adapters/*.json` declaration is schema-validated and discoverable but is not executable merely because it exists. Live Office.js requires a separately authorized host session and exposes only the typed operations declared by that host: Word text/comments/change tracking/content controls, Excel ranges/tables/charts/calculation, and PowerPoint slides/text boxes/geometric shapes. Production deployment pins one exact HTTPS add-in origin and requires a user- or IT-provisioned trusted loopback certificate; Nexa never mutates the certificate trust store. Native release evidence is produced by the protected, SHA-bound Word/Excel/PowerPoint acceptance workflow.

### Office compatibility and PDF operations

For PDF work and Office operations not yet expressed by the typed engine, invoke the bundled Python script through `run_shell`:

```
python <SKILL_DIR>/scripts/edit_doc.py check
python <SKILL_DIR>/scripts/edit_doc.py --path /abs/report.docx replace --find "Q3" --replace "Q4" --dry-run
```

Primary Office commands:

| Need | Command |
|------|---------|
| Create DOCX from body/Markdown/template | `create_docx` |
| Create XLSX from JSON workbook spec | `create_xlsx` |
| Create PPTX from JSON deck spec/template | `create_pptx` |
| Create PPTX from HTML/CSS deck project | `create_html_pptx` |
| Extract text | `extract` |
| Replace/redact text | `replace` / `redact` |
| Snapshot before risky edits | `version` |
| Validate Office/PDF readability | `validate` |
| Convert via LibreOffice | `convert` |
| Render pages/slides to images for QA | `render` |
| Unpack/pack OOXML for precise edits | `unpack` / `pack` |
| Lint XLSX formulas without LibreOffice | `lint_xlsx` |

PPT deep-generation workflows live in the `pptx-presentation-design` skill, not as separate global tools. `create_pptx` remains a compatibility command backed by that skill's native renderer; `create_html_pptx` is the HTML-first route for CSS layout, screenshot QA, hybrid native/raster export, transitions, animations, and deck manifests. For PPT planning, template profiling, style extraction, visual QA, rewrite planning, asset inventory, regression samples, quality gates, and delivery packages, activate the PPT skill and use its bundled scripts/resources.

For generated HTML-first decks, call `create_html_pptx` with `--spec -` and pass the JSON deck spec through `run_shell.stdin`. Do not put raw HTML/CSS/JSON deck content in `run_shell.args`; argv is only for command tokens.

`generate_docx`, `generate_xlsx`, and `ppt_generate` remain registered for compatibility, but they are fallback tools. Prefer `office_artifact` because it supports validation, templates, rendering, formulas, speaker notes, candidate review, and rollback without passing binary content through tool arguments. `create_xlsx` delegates to the XLSX skill renderer for formula fill-down/fill-right, tables, named ranges, validations, conditional formatting, charts, and internal formula QA.

Runtime readiness:

- The desktop app exposes **Settings → Models → Document tools** to check and prepare the Office runtime.
- Preparation creates an app-managed Python virtual environment under the app data directory and installs the bundled `doc-script-editor/scripts/requirements.txt` packages there. It no longer installs or manages Poppler or LibreOffice.
- After preparation, `run_shell` prepends the app-managed Python `Scripts`/`bin` directory to `PATH`, so `python <SKILL_DIR>/scripts/edit_doc.py ...` uses the prepared Office environment automatically.
- If Python itself is not installed, Nexa does not silently install a system runtime. The UI shows the Python download URL and keeps native generators available as simple compatibility fallback.
- LibreOffice and Poppler remain optional system-level applications for conversion and rendering. Excel formula QA uses the internal XLSX linter and does not require LibreOffice.
- Required Python Office packages are exact-pinned. Readiness reports version mismatch instead of treating any newer/older package as equivalent.
- Native PowerPoint rendering uses COM slide export with macros disabled. The Rust tool host applies a 15-minute kill-on-drop watchdog to Python/native Office execution.
- Python Playwright remains optional for HTML-first PPTX screenshot QA. Without it, `create_html_pptx --screenshot auto` still writes HTML, PPTX, manifest, and QA, but reports screenshot coverage as a warning.

---

## 📋 Knowledge Management

### `manage_playbook`

Create, update, list, get details of, add citations to, or delete playbooks — curated evidence collections with annotations.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `action` | string | yes | `create`, `update`, `add_citation`, `list`, `get`, or `delete` |
| `title` | string | no | Playbook title (for create/update) |
| `description` | string | no | Playbook description (for create/update) |
| `body_md` | string | no | Markdown body content (alias for description, for update) |
| `playbook_id` | string | no | Target playbook ID (for get/update/delete/add_citation) |
| `chunk_id` | string | no | Chunk ID to cite (for add_citation) |
| `annotation` | string | no | Annotation text for the citation |

> **Example:** Create a "Production Incident Runbook" playbook and attach evidence chunks from past incident reports.

---

### `submit_feedback`

Upvote, downvote, or pin a search result chunk to train the personalization system for improved future ranking.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `chunk_id` | string | yes | Chunk ID to give feedback on |
| `kind` | string | yes | `upvote`, `downvote`, or `pin` |
| `query` | string | no | Search query context (helps learn per-query relevance) |

> **Example:** Pin a highly useful chunk so it surfaces first in future related searches.

---

## ⚙️ Administration

### `manage_source`

Add, remove, or update a registered source directory. Removing a source deletes
its source record and cascades to its indexed documents and chunks; the original
files on disk are not deleted. Use `update`/`refresh_path` after moving or
renaming a source root rather than removing and recreating its identity.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `action` | string | yes | `add`, `remove`, `update`, or `refresh_path` |
| `path` | string | conditional | Required for add/update/refresh_path |
| `source_id` | string | conditional | Required for remove/update/refresh_path |

> **Example:** Register a new project folder so its documents become searchable.

---

### `reindex_document`

Trigger re-indexing of a specific document or an entire source directory. Use when files have changed or search results seem stale.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | no | File path to reindex (absolute or relative to a source root) |
| `source_id` | string | no | Source ID to reindex entirely |

At least one of `path` or `source_id` should be provided.

> **Example:** Force re-indexing of a document after editing it outside the app.

---

### `fetch_url`

Fetch and extract readable text from a public web page with SSRF and redirect-hop validation. Use after `web_search` or when the user shares a URL and web content needs referencing. HTML pages use a Readability-style article extractor first, then `article`/`main`/`body` fallback. JavaScript-heavy pages are detected and browser-rendered on demand before falling back to metadata.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `url` | string | yes | URL to fetch (http:// or https://) |
| `max_length` | integer | no | Max characters to return (default 5000) |
| `mode` | string | no | `auto`, `readability`, `text`, `metadata`, or `assets` |
| `include_assets` | boolean | no | Include image candidates from metadata, `picture/source`, `srcset`, and `img` tags (default true) |
| `render_js` | string | no | `auto`, `never`, or `always`; default `auto` renders only likely app shells or JavaScript-required pages |

`fetch_url` is text-first. It reports image candidates in artifacts but does not write binary files. If the user wants a candidate image saved, use `download_asset`. Browser rendering keeps the same public URL validation boundary; blocked subrequests are reported in the `jsRender` artifact.

> **Example:** Fetch a Stack Overflow answer the user linked to and incorporate it into the conversation.

---

### `download_asset`

Download a supported public image asset into the workspace. This tool requires confirmation because it writes a file. It validates the URL and each redirect hop, rejects private/local network targets, enforces image MIME allowlists, caps download size, decodes the image before saving, and keeps output paths inside a registered source root or the current workspace.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `url` | string | yes | Public image URL to download (http:// or https://) |
| `output_dir` | string | no | Optional output directory; relative paths are placed under `downloaded-assets` |
| `filename` | string | no | Optional sanitized filename; an image extension is added when missing |
| `max_bytes` | integer | no | Max bytes to download (default 10 MiB, hard cap 25 MiB) |

> **Example:** Save an `og:image` candidate returned by `fetch_url` so the user can inspect or reuse it locally.

---

### `web_search`

Search the public web through Nexa's native no-key providers plus any enabled configured providers such as Brave, Tavily, AnySearch, SerpAPI, or SearXNG. Use it to discover candidate URLs, then use `fetch_url` on the most authoritative results before citing or summarizing them.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `query` | string | yes | One focused, natural-language search query |
| `limit` | integer | no | Max normalized results, 1-20 (default 8; use 10-15 for broad exploration) |
| `region` | string | no | `auto`, `mainland_cn`, or `global` |
| `language` | string | no | `auto`, `zh`, or `en` |
| `engines` | string[] | no | Optional built-in fallback subset of `baidu`, `sogou`, `google`, `bing`, `duckduckgo`; does not override configured provider priority |
| `time_range` | string | no | `any`, `day`, `week`, `month`, or `year`; accepted for provider compatibility |
| `site` | string | no | Optional single-domain filter such as `github.com` |
| `include_snippets` | boolean | no | Include snippets in candidate results (default true) |

Language routing:
- Chinese queries use Baidu first by default, then Sogou/Bing only when needed.
- Provider calls are bounded and may run in small parallel waves; configured custom providers still respect the selected provider priority and fallback mode.
- English queries use Google first by default, then DuckDuckGo/Bing only when needed.
- Avoid stacking unusual operators or several near-duplicate queries. Start with one focused query; use a second query only for a genuinely separate angle.

Use `web_search` for readable search results and `browser_session` for an
interactive page. `desktop_automation` no longer accepts a web-search action.

---

### `browser_session`

Control the conversation-owned Nexa Browser Workspace. This is the canonical
interactive browser surface for agents; the retired built-in Playwright MCP is
not required. The tool shares visible tabs, cookies, control leases, and
observation-scoped element references with the user.

Core actions include session/tab creation and selection, explicit navigation,
back/forward/reload, observation, semantic waits, pointer/keyboard interaction,
and closing tabs or sessions. When a conversation already owns an active
workspace, `sessionId` may be omitted for non-terminal actions. `close_session`
and `close_tab` always require an explicit `sessionId`; `close_tab` also requires
the exact `tabId`. The latest `observationId` and fresh element refs remain
explicit where applicable.

Safety posture:
- Observe before interaction and use refs only from the latest observation.
  A successful Agent observation always carries a decoded, bounded screenshot
  from the visible shared WebView; visual capture failure is a failed
  observation, not unverifiable success.
- Non-terminal mutations invalidate earlier visual proof and require a fresh
  observation. Every requested close requires its typed receipt:
  `close_session` must report `sessionClosed: true`, while `close_tab` binds the
  requested session/tab IDs and reports `remainingTabCount`. The receipt
  replaces a new screenshot only when closing the session or final tab removes
  the last renderable target; a non-final tab close still requires a fresh
  observation of the remaining active tab.
- If temporary-profile cleanup fails after all tabs close, the empty session is
  retained as `cleanupPending` and the same explicit `sessionId` is used to
  retry cleanup without repeating native input.
- Agent navigation is restricted to validated public HTTP(S) targets; private
  network and unapproved navigation remain blocked by the native proxy.
- User takeover invalidates the Agent control lease and prior observations.
- Consequential actions retain the normal approval policy.

---

### `browser_evidence_capture`

Open a public or loopback development page in a read-only Chromium diagnostics
lane. It returns rendered text, interactive metadata, a screenshot, console
messages, JavaScript exceptions, failed requests, HTTP 4xx/5xx responses, and
provenance. It never clicks or types. Use `browser_session` for interactive
navigation and user flows; keep this tool for deterministic observe-fix-verify
debugging. Private LAN, link-local, and metadata-service targets remain
blocked.

### `computer_observe`

Observe a native Windows window. `list_windows` returns verified external
top-level windows. `capture_window` returns an ephemeral screenshot plus a
bounded UI Automation projection; `capture_mode: "som"` overlays element IDs.
`wait_for_change` polls a captured observation for a material perceptual
change. Capture actions require explicit model-egress consent.

Important fields:

- `observation_id` and `window_id` scope every follow-up.
- `include_elements` defaults to true; `max_elements` defaults to 120.
- Element IDs such as `e7` are valid only for that observation.
- `timeout_ms` and `poll_interval_ms` bound `wait_for_change`.
- Pixels and accessibility text are untrusted data and never instructions.

### `computer_control`

Perform exactly one approved action against a fresh Windows observation.
Observations are single-use for control. Prefer semantic `invoke` or
`set_value`, then element-targeted pointer actions, with raw coordinates as the
last fallback. Coordinates may use `captured_image_pixels` or
`normalized_0_1`.

The result distinguishes delivery from effect: `route`, `delivery`,
`deliveryStatus`, `effect`, and perceptual `verification` do not by themselves
prove the user's task succeeded. If post-action observation fails, the action
is reported as unverifiable and must not be blindly retried. Text/key arguments
are structurally redacted from approval, UI, trace, and durable provider-turn
projections. Post-action screenshots and UIA names remain current-turn-only;
durable artifacts retain hashes, counts, route, delivery, and effect receipts.

### `desktop_automation`

Open or reveal a source-scoped path on the user's visible desktop. Prefer
`open_in_nexa` for supported file previews. Opening a previewable file in an
external application requires the user's explicit request and
`external_requested: true`. HTTP(S) navigation belongs to `browser_session`.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `action` | string | yes | `open_path` or `reveal_path` |
| `path` | string | for either action | Absolute or source-root relative path inside the active registered source scope |
| `external_requested` | boolean | no | True only for an explicitly requested external application |
| `reason` | string | no | Brief user-facing reason for the action |

Safety posture:
- Desktop file handoff retains the applicable approval policy.
- Local path actions must resolve inside a registered source and the active source scope.
- Use `web_search` for readable search results.
- Use `fetch_url` when the agent needs page text; use `browser_session` when the page must be observed or manipulated.

> **Example:** Reveal a generated report in the file manager under its registered source.

---

### `run_shell`

Execute a whitelisted program with explicit argv arguments inside a registered source directory. The program is spawned directly — **there is no shell interpreter**, so metacharacters like `;`, `&&`, `|`, backticks, and globs are passed literally and never interpreted.

File-change previews for native `cp` and `mv` cover their resolved mutation paths.
Other commands do not scan or hash the workspace before/after execution, and
background processes do not retain workspace contents. Their exit status and
output are process receipts; an absent diff does not prove files were unchanged
or verified. Use explicit `git diff`, file reads, or format-specific checks when
verifying generated/modified artifacts, and record that verification. Untracked
shell effects remain pending in the runtime evidence audit until verified.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `program` | string | yes | Program basename; must be in the whitelist |
| `args` | string[] | no | Argv list passed to the program (no shell expansion) |
| `cwd` | string | yes | Working directory (absolute or relative to a source root) |
| `timeout_secs` | integer | no | Timeout in seconds (default 30); `0` disables the per-command timeout for intentional long installs/downloads/builds |

**Default restricted whitelist:** `python`, `python3`, `pip`, `pip3`, `node`, `npm`, `npx`, `git`, `pwd`, `ls`, `cat`, `mkdir`, `cp`, `mv` (`pip`/`pip3` are normalized to `python -m pip` / `python3 -m pip`; `copy`/`move` aliases normalize to `cp`/`mv`). `git` is read-only by default: allowed subcommands are `status`, `diff`, `log`, `show`, `ls-files`, `rev-parse`, `branch`, `tag`, `config`, `remote`, `describe`, and `blame`. `git config` additionally requires an explicit read-only flag such as `--get`, `--list`, or `--get-regexp`. In less-restricted Shell Access modes, arbitrary bare command names (for example `bash` or `powershell` when available) may be allowed, but `run_shell` still does not invoke a shell automatically.

**Safety posture:**
- Always requires user confirmation before executing.
- stdout and stderr are each capped at 64 KB.
- Default timeout 30s; `timeout_secs: 0` disables the per-command timeout for intentional long installs/downloads/builds. The broader agent turn timeout can still stop the run unless it is also raised or disabled. Timed-out processes are killed.
- Environment is rebuilt from scratch: secret-like vars (`*KEY*`, `*SECRET*`, `*TOKEN*`, `*PASSWORD*`, `*CREDENTIAL*`, …) are stripped; only a neutral allow-list (`PATH`, `LANG`, `HOME`, …) is forwarded.
- `cwd` must canonicalize inside a registered source root (path sandbox).
- No stdin is attached; interactive programs cannot prompt.
- No network tunneling is provided — blocking network I/O is up to the child program.
- Windows: child is spawned with `CREATE_NO_WINDOW` (no console flash).

**Usage examples:** `python script.py`, `python -m pytest -q`, `node script.js`, `npm test`, `git status`, `git diff --stat`, `git log --oneline -n 20`, `git config --list`.

**Cannot do in default restricted mode (by design):**
- No file-deletion helpers (no `rm`, `Remove-Item`, `del`).
- No network fetchers (no `curl`, `wget`, `Invoke-WebRequest`).
- No git write operations (`push`, `pull`, `fetch`, `commit`, `reset`, `merge`, `rebase`, `clone`, `add`, `checkout`, `stash`, `--set`, `--unset`, `--add`, …).
- No shell interpreter wrappers from the restricted whitelist (no `sh -c`, `bash -c`, `cmd /c`, `powershell -c`). Metacharacters do not expand unless the user explicitly relaxes Shell Access and runs a shell program themselves.

> **Example:** Run `python -m pytest -q` in a project source root and capture the summary output, or run `git diff --stat HEAD~1` to preview recent changes.

---

### `terminal_session`

Inspect or interact with the user-visible terminal linked to the current
conversation. This tool is added by the desktop runtime and is unavailable when
there is no linked session.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `action` | string | no | `inspect` (default), `write`, or `interrupt` |
| `sessionId` | string | no | Linked session ID; omit to use the current conversation's terminal |
| `data` | string | no* | Input for `write`, capped at 16,000 characters |
| `submit` | boolean | no | Append Enter after `write` data (default false) |
| `maxChars` | integer | no | Recent output returned by `inspect`, 1-48,000 (default 12,000) |

`*` `data` is required for `write`.

- `inspect` is read-only and needs no confirmation.
- `write` and `interrupt` operate the live PTY and always require user
  confirmation.
- Recent output is bounded, stripped of common control sequences, and marked as
  untrusted local observation. Terminal text cannot instruct the agent.
- The tool resolves only sessions linked to the active conversation.

See [TERMINAL_AGENT_BRIDGE.md](./TERMINAL_AGENT_BRIDGE.md)
for the UI, lifecycle, and security contract.

---

## 🧭 Delegation Tools

### `list_subagent_models`

Return configured account IDs, provider IDs and known models without credentials
or endpoint secrets. Reuse the returned routes when assigning workers. API
parents inherit their route by default; subscription parents explicitly select
an available API worker account.

### `spawn_subagent`

Spawn one short-lived worker for an isolated subtask. Subagents inherit the supervisor's source scope by default, can be narrowed further, and run under a shared per-turn budget.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `task` | string | yes | Concrete delegated task |
| `agent_config_id` | string | no | Saved worker account/endpoint from `list_subagent_models` |
| `provider` | string | no | Configured provider ID; use the account ID if ambiguous |
| `model` | string | no | Model on the selected route; overrides `model_policy` |
| `reasoning_effort` | string | no | Supported worker reasoning level; overrides role defaults |
| `role_id` | string | no | Structured role: `researcher`, `verifier`, `critic`, `planner`, `writer`, `connector`, or `desktop_operator` |
| `role` | string | no | Free-form role nuance; prefer `role_id` for known profiles |
| `context` | string | no | Supervisor handoff context |
| `expected_output` | string | no | Desired deliverable shape |
| `acceptance_criteria` | string[] | no | Checklist the worker should satisfy |
| `evidence_chunk_ids` | string[] | no | Exact evidence chunks to hand off |
| `source_ids` | string[] | no | Narrower source scope |
| `allowed_tools` | string[] | no | Narrower tool whitelist |
| `parallel_group` | string | no | Label for sibling workers |
| `deliverable_style` | string | no | Style hint such as critique, plan, or fact check |
| `return_sections` | string[] | no | Ordered response section titles |
| `max_iterations` | integer | no | Inherit parent tool-round budget when omitted; zero is answer-only |
| `timeout_secs` | integer | no | Positive deadline bounded by the configured delegation run deadline |

Role profiles set default return sections, timeout estimates and recommended tool
subsets when `allowed_tools` is omitted. Explicit tool lists can select any
parent-granted tool, and `[]` grants no tools. The six-round cap and 180-second
explicit argument cap no longer override worker configuration. Shared call,
token, concurrency and run-deadline budgets still apply. Workers run their own
handoff policy without inheriting the parent's Nexus fan-out requirements.

Delegation remains one level deep: nested workers would need scheduler support
to release a waiting parent's concurrency permit. Interactive browser/computer
tools remain parent-owned until workers have scoped surface leases and approval
proxies. These unavailable tools are excluded from worker discovery.

### `spawn_subagent_batch`

Launch several workers under one shared budget. Provide explicit `tasks`, or provide a `workflow_template` plus `batch_goal` and let Nexa expand the batch.

Each task accepts its own account, model and reasoning selection. A batch can
contain up to 32 workers, with up to 12 concurrent workers subject to the user's
shared limits. Oversized batches fail explicitly instead of dropping tasks.

Built-in workflow templates:

| Template | Workers | Use |
|----------|---------|-----|
| `research_verify` | researcher, verifier, critic | Evidence gathering plus independent verification |
| `draft_review` | writer, critic, verifier | Draft creation, critique, and fact check |
| `connector_background` | connector, planner, verifier | Connector setup, background-task lifecycle, and safety review |

Batch results include each worker's role, tool scope, evidence handoff, usage, and errors so the supervisor can synthesize or adjudicate explicitly.

### `judge_subagent_results`

Run a separate adjudication pass over two or more delegated results. Use it when parallel workers disagree, when a rubric matters, or when the final answer should cite why one candidate was selected.
