# Experimental Code Mode

Nexa can run a Luau script that calls tools, uses one result to decide what to call next, and returns just the answer to the model. For example, the agent can find a report, open it and count matching rows in a single model tool call. The individual reads still appear in the activity trace.

## Turn it on

On desktop, open **Settings → Appearance → Agent behavior**, enable **Code Mode (experimental)** and save. For chats running on Android, enable the same switch in the phone settings under **Agent behavior**. A paired computer uses its own setting. The option defaults to off. Enabling it changes the schema for subsequent turns. Disabling it also prevents queued and future cells from starting. A workflow must already grant access to `context_history`. [Desktop registration](../../apps/desktop/src-tauri/src/desktop_agent_session/dependencies.rs) · [Phone registration](../../apps/mobile/src-tauri/src/local.rs)

## Call live tools

The model selects `context_history` with `action="code_mode"` and supplies the script in `code`. The normal turn dispatcher checks every tool call against the current host's registry, workspace, permissions and privacy settings. A tool failure stops the cell and returns an error to the model. Explicit tool-call budgets on external sessions count the individual calls as well as the cell.

```lua
local listing = call_tool("list_dir", {path = "."})
local file = string.match(listing, "ledger%-%x+%.txt")
if not file then return "No matching ledger" end
local content = call_tool("read_file", {path = file})
local _, count = string.gsub(content, "invoice amount", "")
return count
```

- `list_tools()` returns the live tools available on this host for this cell. `tool_schema(name)` returns the tool's argument schema on demand.
- `call_tool(name, arguments)` returns the tool's model-facing text. The same script can issue another call using that text.
- `call_tools({{name="read_file", arguments={path="a.txt"}}, {name="read_file", arguments={path="b.txt"}}})` schedules independent reads in small groups and returns their text in input order.

The live bridge supports local file, document, knowledge, health and memory read operations. The actual arguments must qualify for local text access. Shell commands, writes, remote connectors, native images and interactive actions use their normal tools. The final script value enters the model's conversation. Full intermediate text remains available in the privacy-filtered activity trace. A script saves context when it filters verbose results. A short ordinary tool call can cost fewer tokens than the script that wraps it.

In a local comparison, both paths listed a directory, found a generated ledger filename and counted 80 entries in that file. The GPT-4o tokenizer estimated 1,475 tokens for the ordinary tool arguments and results. The script and final count used 86, with another 151 for the enabled schema. The fixture uses temporary paths, so the ordinary count varies slightly between runs. The comparison excludes shared prompts, provider caching and model reasoning. [Comparison fixture](../../crates/core/src/agent/external_tools.rs)

## Read earlier results

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

Each call starts a fresh interpreter. Save counts and cursors in the conversation for a long analysis. A cell accepts 64 KiB of source, has a 16 MiB VM heap and a 30-second deadline, and can return up to 1 MiB of text. At most two cells run at once per host process. Queued cells can be cancelled. The overall task can continue across cells. [Cell implementation](../../crates/core/src/tools/context_history_tool/code_mode.rs) · [Luau sandbox design](https://luau.org/sandbox/)

## Upstream implementations checked on 2026-11-25

| Project | Published behavior and implications for Nexa |
| --- | --- |
| Pi 1.6.0 | The official coding agent supports `pi --tools +codemode`. JavaScript runs in QuickJS and invokes the session’s tools through a bridge. Scripts can filter intermediate output and store small values for later calls. The separately documented durable extension can resume scripts after restart. [CLI guide](https://github.com/earendil-works/pi/blob/v1.6.0/packages/coding-agent/docs/cli.md#enable-codemode) · [Coding-agent implementation guide](https://github.com/earendil-works/pi/blob/v1.6.0/packages/coding-agent/docs/codemode.md) · [Durable agent](https://github.com/earendil-works/pi/blob/v1.6.0/packages/durable/README.md#code-mode) |
| Claude Code 2.1.332 | Anthropic documents an opt-in JavaScript tool mode from 2.1.323 with transcript inspection and separate hook, permission and background-execution rules. Its November 25 release addresses skipped calls, large inputs and interrupted background work. The product docs and changelog establish this behavior without relying on unofficial source reconstructions. [Product documentation](https://code.claude.com/docs/en/code-mode) · [Release notes](https://github.com/anthropics/claude-code/releases/tag/v2.1.332) |
| OpenAI Codex | Its published protocol delegates nested calls to the session owner. Sessions own active cells, and the protocol includes execution, wait and termination. Full live orchestration therefore needs a host-level lifecycle and approval path. [Session protocol](https://github.com/openai/codex/blob/bede3b5df8dfcdfa4ff74c8d762f1fb72ccaff9e/codex-rs/code-mode-protocol/src/grpc/codex.code_mode.v1.proto) |
| DeepSeek Harness | Its programmatic executor derives an SDK from scoped tools and exposes `run_code` to the model. It accounts for nested calls and parallel-safe concurrency. [Official executor](https://github.com/deepseek-ai/deepseek-harness/blob/4b399c39a6d1479cd84ca591b4d7c88e24d38a96/packages/core/tool-executor-ptc/README.md) |
| Hermes Agent | `execute_code` runs Python with an RPC bridge to an allowlisted group of tools and returns printed results. Its interpreter and working directory depend on execution mode. This directly couples the feature to a Python host, unlike Nexa’s embedded VM on Android. [Official code execution guide](https://hermes-agent.nousresearch.com/docs/user-guide/features/code-execution) |
| Zed | The native agent publishes individual file, terminal and search tools with separate profiles and permissions. The inspected native tool inventory contains no general programmatic composition tool. Its separation of availability and approval applies to any future Nexa live-call bridge. [Native tool inventory](https://github.com/zed-industries/zed/tree/2664a0aa1eae5dabf8f8273c03f88d53061ca7dc/crates/agent/src/tools) · [Tool permissions and profiles](https://zed.dev/docs/ai/tools) |

Nexa routes live script calls through the same admission and execution path as ordinary model tool calls. Each child receives a host-owned identity and a trace receipt. Only the parent cell creates a tool result in the provider conversation. [Turn dispatcher](../../crates/core/src/agent/tool_dispatch.rs) · [Tool bridge](../../crates/core/src/tools/context_history_tool/live.rs)
