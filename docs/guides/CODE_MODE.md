# Experimental Code Mode

Nexa can use a local Luau script to answer questions about tool results already recorded in a conversation. For example, it can count matching entries across several long search outputs and return the count in one tool call. The feature uses the existing `context_history` tool on the host running the chat.

## Turn it on

On desktop, open **Settings → Appearance → Agent behavior**, enable **Code Mode (experimental)** and save. For chats running on Android, enable the same switch in the phone settings under **Agent behavior**. A paired computer uses its own setting. The option defaults to off. Enabling it changes the schema for subsequent turns. Disabling it also prevents queued and future cells from starting. A workflow must already grant access to `context_history`. [Desktop registration](../../apps/desktop/src-tauri/src/desktop_agent_session/dependencies.rs) · [Phone registration](../../apps/mobile/src-tauri/src/local.rs)

## Read and combine results

The model calls `context_history` with `action` set to `code_mode` and a `code` string. `list_results()` returns identities from the most recent turn. Its `results` array includes the turn ID, call ID, tool name and any persisted result ID. Follow `nextTurnId` and `nextOffset` to enumerate earlier pages or turns. `read_result(turnId, toolCallId, resultId, offset, expectedDigest)` uses the same scoped lookup and Unicode offsets as `read_tool_result`. It returns the current privacy-projected model text. The IDs and digest protect against reused call IDs and changes between pages. [History reader](../../crates/core/src/context_history.rs)

This cell counts bytes across all pages of one recorded output. A script can select multiple identities, parse JSON with `decode_json`, compare fields and return a scalar. JSON null is available as `json_null`.

```lua
local refs = list_results()
while not refs.results[1] and refs.nextTurnId do
    refs = list_results(refs.nextTurnId, refs.nextOffset)
end
local r = refs.results[1]
if not r then return 0 end
local offset, digest, bytes = 0, nil, 0
repeat
    local page = read_result(r.turnId, r.toolCallId, r.resultId, offset, digest)
    bytes += #page.text
    offset, digest = page.nextOffset, page.contentDigest
until offset == nil
return bytes
```

Each call starts a fresh interpreter. Save intermediate counts and cursors in the conversation for a long analysis. The cell accepts 64 KiB of source, has a 16 MiB VM heap and a 30-second execution budget, and can return up to 1 MiB of text. Two cells can execute at once per host process. Additional calls wait for a slot and can be cancelled while waiting. These are per-call budgets. The experiment adds no deadline or cell-count limit to the overall task. Its only host data functions enumerate and read existing results. Filesystem, shell, connectors, UI actions and model calls continue through their ordinary tools and permissions. [Cell implementation](../../crates/core/src/tools/context_history_tool/code_mode.rs) · [Luau sandbox design](https://luau.org/sandbox/)

## Upstream implementations checked on 2026-11-25

| Project | Published behavior and implications for Nexa |
| --- | --- |
| Pi 1.6.0 | The official coding agent supports `pi --tools +codemode`. JavaScript runs in QuickJS and invokes the session’s tools through a bridge. Scripts can filter intermediate output and store small values for later calls. The separately documented durable extension can resume scripts after restart. [CLI guide](https://github.com/earendil-works/pi/blob/v1.6.0/packages/coding-agent/docs/cli.md#enable-codemode) · [Coding-agent implementation guide](https://github.com/earendil-works/pi/blob/v1.6.0/packages/coding-agent/docs/codemode.md) · [Durable agent](https://github.com/earendil-works/pi/blob/v1.6.0/packages/durable/README.md#code-mode) |
| Claude Code 2.1.332 | Anthropic documents an opt-in JavaScript tool mode from 2.1.323 with transcript inspection and separate hook, permission and background-execution rules. Its November 25 release addresses skipped calls, large inputs and interrupted background work. The product docs and changelog establish this behavior without relying on unofficial source reconstructions. [Product documentation](https://code.claude.com/docs/en/code-mode) · [Release notes](https://github.com/anthropics/claude-code/releases/tag/v2.1.332) |
| OpenAI Codex | Its published protocol delegates nested calls to the session owner. Sessions own active cells, and the protocol includes execution, wait and termination. Full live orchestration therefore needs a host-level lifecycle and approval path. [Session protocol](https://github.com/openai/codex/blob/bede3b5df8dfcdfa4ff74c8d762f1fb72ccaff9e/codex-rs/code-mode-protocol/src/grpc/codex.code_mode.v1.proto) |
| DeepSeek Harness | Its programmatic executor derives an SDK from scoped tools and exposes `run_code` to the model. It accounts for nested calls and parallel-safe concurrency. [Official executor](https://github.com/deepseek-ai/deepseek-harness/blob/4b399c39a6d1479cd84ca591b4d7c88e24d38a96/packages/core/tool-executor-ptc/README.md) |
| Hermes Agent | `execute_code` runs Python with an RPC bridge to an allowlisted group of tools and returns printed results. Its interpreter and working directory depend on execution mode. This directly couples the feature to a Python host, unlike Nexa’s embedded VM on Android. [Official code execution guide](https://hermes-agent.nousresearch.com/docs/user-guide/features/code-execution) |
| Zed | The native agent publishes individual file, terminal and search tools with separate profiles and permissions. The inspected native tool inventory contains no general programmatic composition tool. Its separation of availability and approval applies to any future Nexa live-call bridge. [Native tool inventory](https://github.com/zed-industries/zed/tree/2664a0aa1eae5dabf8f8273c03f88d53061ca7dc/crates/agent/src/tools) · [Tool permissions and profiles](https://zed.dev/docs/ai/tools) |

Nexa currently admits live calls in the turn dispatcher, where approval, cancellation, trace identity and workflow scope are enforced. Routing a script directly to the registry would skip parts of that path. The experiment therefore composes historical reads. Extending it to live calls requires a dispatcher that gives every nested call the same admission, trace, cancellation and replay behavior as a call issued separately by the model. [Turn dispatcher](../../crates/core/src/agent/tool_dispatch.rs) · [Existing tool contract](../../crates/core/src/tools/context_history_tool.rs)
