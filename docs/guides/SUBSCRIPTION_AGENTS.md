# External agents and subscriptions

[Documentation](../README.md) / [Category overview](README.md)

Settings → AI Providers separates **API models** from **External agents**.
Add **GitHub Copilot** or **ChatGPT / Codex** from External agents → Add Provider.
Sign in through the official runtime and save the provider.
Choose the model and reasoning level in Chat. The saved provider and its currently available models
then appear in the Chat model picker. Reasoning levels come from that account's
model catalog. Signing in does not automatically create a provider.

Copilot discovery and execution use the same editor integration catalog through
the official SDK. The bundled CLI is pinned to `1.0.100` with upstream archive
hashes; `status.get` reports the actual SDK runtime version (a standalone
`--version` command may follow a newer locally installed CLI). Gemini 4 Argon's
reasoning levels are read from the account catalog and passed to the session;
Nexa does not infer them from the model name. Explicit CLI path overrides remain
user-managed and need a version that supports the selected model.

The gauge beside the Chat model and reasoning controls opens **Account usage**.
It shows provider-reported used and remaining percentages, individual quota
windows and reset times. Settings uses the same quota presentation. Opening or
refreshing the panel reads the account; while it remains visible it refreshes
once per minute without overlapping requests. Closing it stops polling. A failed
refresh clears old percentages, and changing profiles discards late responses.
Transient read failures retry on the same visible-panel schedule.
Chat and Settings share bounded in-flight account reads. A stalled read settles
as unavailable with Refresh enabled; late responses cannot replace a newer read.
Codex version probes and process trees are bounded, and optional cumulative token
statistics do not delay quota. Copilot quota and model discovery run independently
within the account read, so a stalled model catalog does not discard valid quota.

Copilot Context policy offers the native `default` and, only when the current
account catalog advertises it, `long_context` tier. The choice is saved through the
existing endpoint/model policy and applies on the next turn. Prompt limits are
shown as prompt limits, not invented total context windows; the active runtime's
usage event remains the authority for occupancy. Native runtimes continue to own
their compaction thresholds.

Background compaction and foreground streaming can coexist. ACP negotiates
`session.compaction` and projects compaction IDs and terminal statuses separately
from answer/tool activity. Compaction summaries never become assistant answers;
notices do not terminate prompts. Native model changes update usage identity,
while the composer selection only configures the next request.

| Connection | Account quota source |
| --- | --- |
| ChatGPT / Codex | Official app-server `account/rateLimits/read`, including all returned buckets |
| GitHub Copilot | Official SDK `account.getQuota` |
| Hermes | Installed `hermes usage --json --provider <provider>` with the saved executable/environment and the provider encoded in the selected ACP model |
| Other ACP connections | Explicitly unavailable until the runtime exposes a supported account-usage contract |

Hermes needs a version that implements `usage --json`. Only its standard `acp`
argv and unmodified native config options are supported for this query; custom
wrappers and ambiguous model/provider identities show unsupported instead of
reading a different account. Quota probes resolve the current conversation's
workspace/worktree (or the selected project's workspace for a draft), exactly as
ACP execution does. A new projectless chat without a configured directory waits
until its conversation directory exists; it never substitutes a discovery folder.
This also keeps relative saved or inherited `HERMES_HOME` values on the same
account. CLI credentials remain owned by the CLI. A Hermes
credential pool may select a different entry for an active turn; this panel
reports the configured profile account returned by the standalone usage command.
Missing percentages remain unknown; unlimited entitlement is a label rather than
a fabricated 100% gauge. Context occupancy and the chat's token/cost totals do not
measure subscription quota, and neither implies that an ACP agent has a paid plan.

External-agent icons are bundled SVGs keyed by runtime identity, so selecting a
GPT or Claude model inside pi, Hermes, Copilot or another agent keeps that agent's
icon. The reasoning slider uses discrete provider-advertised options, a circular
handle and Nexa's paired flowing strands. List mode remains available; both modes
save the same model and reasoning settings, and reduced motion disables the flow.

For API/local connections and endpoint capability resolution, see
[Models and providers](PROVIDERS_AND_MODELS.md). Subscription support follows
the bundled driver contract; it is not a promise of parity with every upstream
runtime feature or account plan.

## Execution ownership

`DesktopAgentBackend` selects Nexa's direct API executor in `core::llm` or an
externally owned loop in desktop `agent_runtime`. The latter contains separate
Copilot, Codex and ACP adapters. An external agent is not an HTTP model endpoint;
it can use a subscription, its own API connection, or a local model.
Copilot's SDK and Codex's app-server own their model loops. They use the same
Nexa tool dispatcher, approval callback, activity database, cancellation token,
browser observation fence, message persistence and ordered Run Event outbox as
direct chat. Tools retain their actual names and schemas.

Copilot and Codex create one upstream session per Nexa turn; ACP can reuse a completed session. The driver remains in the backend
when the renderer reloads; reconciliation reads the existing outbox instead of
submitting another upstream turn. Nexa's bounded reference history is supplied
as reference context, without replaying another provider's native tool records.
Historical skill mentions are outside new user input. The upstream runtime owns
compression within its session; Nexa remains the owner of cross-turn history.

Copilot/Codex tool callback IDs are idempotent within the live session. Reusing an ID with
different arguments fails the run. Tool effects are serialized through the
shared dispatcher; invalid schema and denied actions produce structured errors
without an effect. The configured tool budget also applies to external callbacks
(automatic has no hidden lifetime call limit). Completed receipts spill into a
private temporary database so long runs retain idempotency without accumulating
all callback payloads in memory. Renderer reload never replays
callbacks. Process loss terminates the turn; it does not transparently resend
an uncertain action.

## Installed ACP agents

The bundled External agents catalog includes 41 supported official agents,
plus Hermes and a custom ACP entry. Settings reads the
[official ACP directory](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json)
through one host-owned cache, checking at most hourly unless explicitly refreshed.
Directory discovery and version activation are separate: each agent keeps its
selected descriptor until **Use version** prepares the candidate and verifies its
ACP handshake. A failed refresh or candidate keeps the previous selection; new
agents also retain their first discovered version. Running sessions keep their
existing process. Model/account discovery remains the agent runtime's authority.
Common CLI entrypoints include:

| Agent | Command | Preparation |
| --- | --- | --- |
| Gemini CLI | `gemini --acp` | Install the official CLI and complete its login |
| OpenCode | `opencode acp` | Install and configure its native providers/account |
| Hermes Agent | `hermes acp` | Install with the ACP extra and configure its native account |
| GitHub Copilot CLI (ACP) | `copilot --acp` | Use the official CLI login; this route uses native Copilot tools |
| Claude Code (ACP) | `claude-agent-acp` | Install the Agent Client Protocol Claude adapter and authenticate Claude Code |
| Codex CLI (ACP) | `codex-acp` | Install the Agent Client Protocol Codex adapter and authenticate Codex |
| Qwen Code | `qwen --acp` | Install and authenticate the official CLI |
| Goose | `goose acp` | Install and configure a native provider |
| Auggie | `auggie --acp` | Install and authenticate the Augment CLI |

**Download / check connection** prepares the selected directory version using
its exact `npx`/`uvx` package or official platform archive. An explicit executable
override remains user-managed. Publisher SHA-256 values are verified when
present. Only descriptors obtained from the official HTTPS directory may omit
that pin; Nexa records the observed archive digest and validates the extracted
file tree on reuse. This receipt detects local changes but is not a publisher
signature. Refresh retains reviewed pins only for an unchanged archive URL and
entrypoint. Offline startup uses the last good cache and bundled fallback.
For automatic package preparation, custom arguments belong to the adapter;
Nexa supplies the pinned package invocation. A previously displayed complete
runner prefix for the same package, including its older version, is normalized
to the current reviewed pin without duplication. Explicit executable
overrides retain their complete process argument contract.
Archive installation preserves companion executables and publishes only a
complete extracted directory.
Cached installations are checked against a host-created receipt covering every
extracted file, permission mode and safe link. Missing helpers, changed bytes,
unexpected files or an old/incomplete receipt trigger a fresh pinned download
and atomic replacement; a failed publication restores the preceding directory.
A missing
package runner or unsupported platform produces an actionable setup error.
**Advanced** accepts a JSON argv
array and environment object, including custom ACP servers. These preferences
are encrypted at rest; legacy preferences remain readable. Windows batch shims
reject shell metacharacters in custom arguments; use a native executable when
literal arguments require those characters. Registry inclusion proves a launch
contract, not account access or every upstream feature. External ACP wire
negotiation remains version 1; Zed's private internal protocol is separate.

The working directory follows the current chat automatically. A chat worktree
or project's primary folder takes priority. A projectless chat gets a stable,
separate folder under the application data directory's `workspaces/external-agents`;
catalog discovery uses its own managed folder. An explicitly empty or unavailable
project workspace remains an actionable error rather than selecting another folder.
An optional custom fallback directory is available under **Advanced**, and an
absolute executable path can be supplied when the CLI is not on PATH.
Launch preferences are stored separately from API credentials under the exact
saved profile ID. Editing one profile cannot redirect another profile. A project's
primary workspace folder overrides the profile's fallback directory. The process
cwd and ACP session cwd always agree; Nexa never infers an ACP cwd from its own
process directory. On Windows the launcher respects an explicitly configured
PATH, otherwise includes current registered and common per-user CLI paths. Known
npm shims use their Node entry point to preserve literal argv. Adapter preparation
is part of connecting the selected profile; sign-in requires a separate action.

When a server requires authentication, Settings presents its advertised methods.
Agent methods use the ACP authentication request. Terminal methods open the same
adapter in Nexa's existing terminal, and a successful exit triggers a new connection
check. Editing or closing setup cancels outstanding work. Startup errors include a
bounded, redacted stderr tail, while credential values remain encrypted at rest.
The explicitly selected agent method is saved with the profile and reused when a
fresh process requests authentication. Nexa never selects a method automatically
or invokes terminal login as part of a chat connection.
If that saved attempt is rejected or its process fails, Settings keeps the
advertised recovery choices available. An explicit attempt never retries an old
method, and its acknowledgement counts as success only after session creation.
Changing the selected method participates in the unsaved-settings guard.

Antigravity ACP support is suspended. Its adapter and Google-specific setup have
been removed; directory refresh and cached entries cannot re-enable it.

If session creation instead fails with a generic configuration error, Settings
retains both the diagnostic and the advertised native setup methods. This does
not classify the error as an authentication failure, mark the connection as
verified or permit saving an unverified selection. Runtime chat still propagates
the failed session. RPC diagnostics include bounded redacted messages and stderr,
retaining the final cause of long startup tracebacks without exposing arbitrary
error payloads.

Transient connection and configuration handshakes reconnect through the same
five-attempt backoff policy used by Nexa's model connections. Cancellation stops
the wait immediately. A native prompt is never blindly replayed after tool effects
may have started: interruptions retain the existing resumable checkpoint and
partial output. Native Codex retry notices remain visible while its own loop is
recovering. An incomplete Nexa tool callback returns a validation result to that
loop for correction, with no effect or malformed transcript record; normalized
arguments share the same execution receipt for replay.

A first package-runner installation shares the archive installer's ten-minute
startup window; later session/authentication operations keep their own deadlines.
Closing setup cancels the owned process tree. Reviewed `runnerArgs` belong to the
package manager, while profile arguments still belong to the ACP adapter. For
example, Minion Code 0.1.44 is constrained to its compatible Python ACP SDK rather
than resolving an incompatible newer SDK on every fresh install. Registry import
preserves such a constraint only while the exact package pin remains unchanged.

The Windows audit on 2026-10-13 exercised all 43 presets through real adapters
with isolated homes. Gemini, Claude, Copilot and OpenCode created sessions;
Codex, Qwen and Pi reached their advertised authentication boundary. Pi still
requires its separate CLI (the audit used 0.81.0 with pi-acp 0.0.34); installations
that expose only `pi.exe` can set the adapter's `PI_ACP_PI_COMMAND`. Hermes 0.21.7
with ACP extras initialized and exposed its terminal setup method; an empty
provider configuration returned a session error, not a verified connection.
These checks did not request authentication or inference.

Some published registry entries have upstream limitations that a successful
download cannot resolve:

| Audited release | Confirmed limitation |
| --- | --- |
| Agoragentic 1.3.0 | The published package omits authentication IDs and the ACP chat session methods; it exposes MCP-style tool methods instead. |
| Cortex Code 1.0.73 Windows | Its archive includes Unix venv links, including `/usr/bin/python3.12`; Nexa does not bypass archive containment checks. |
| Devin 3000.11.3 Windows | The isolated process fails to determine its config directory despite an explicit APPDATA. The cause is unresolved, not assumed to be authentication. |
| Harn 0.10.165 | Session admission requires a nonstandard `environmentPolicy` field. Nexa does not invent an environment grant. |
| Minion Code 0.1.44 Windows | The SDK constraint restores imported APIs, but its global stdout redirection sends the older SDK's Windows protocol writer to stderr. A publisher fix is still needed for Windows ACP. |

Settings keeps genuine startup/session errors visible. A completed handshake,
native setup availability and a created session are distinct outcomes; none
alone verifies account quota or a model response.

**Check connection** runs `initialize` and `session/new` without a prompt. It
loads the real catalog, not a hard-coded cloud model list. Success means the
session was created; it does not prove an account has quota or can infer.
Config-options model selection is preferred when advertised; legacy
`models`/`session/set_model` remains supported. Model IDs are opaque and an
unconfirmed change fails before submission. A runtime without model discovery
can expose only its own default. API keys/endpoints cannot be saved on a runtime
profile, and unavailable models never silently switch providers.

Native select options are discovered, including mode, provider and reasoning.
Provider/mode changes are applied before model selection, and model-dependent
options are refreshed afterward. Successful native responses replace the complete
option list; normalized values (such as Qwen's default reasoning level) are kept.
Reasoning controls appear below the chat composer, with a list or intensity
slider presentation. Both send the same advertised option IDs, including opaque
ACP values; Settings no longer duplicates the reasoning toggle.
The final model must still match the user's selection. Model-dependent preferences
are bound to the model verified in Settings, so a chat model switch cannot replay
obsolete effort/Fast options. Discovery returns replacement choices after a saved
model retires; inference rejects it until the user selects a replacement.
Available native slash
commands are sent verbatim without a Nexa instruction wrapper; running a command
in a fresh session does not consume the pending project context for its next prompt.
Namespaced commands retain their prefix. Silent command completion emits a typed
status receipt without inventing an assistant reply. Queued commands and ordinary
steering messages execute as separate prompts so the latter are not swallowed.

Profiles can explicitly select enabled user-managed MCP connectors to forward.
Stdio is supported by ACP; HTTP/SSE requires the agent's advertised capability.
Selected connector environment variables/headers are passed to the native agent,
which owns execution and permissions. Built-in Nexa connectors are not forwarded.

ACP tools execute in the external process with its native configuration and
permission policy. Nexa projects `tool_call` reports with `providerExecuted` and
does not run them again. ACP permission requests use Nexa's approval UI, bind to
the profile, working directory and stable action arguments, and select only a
corresponding one-time option. Transient RPC/session IDs do not invalidate a
reusable Nexa decision, but incomplete action details remain invocation-specific.
A reusable Nexa decision is never promoted into an upstream `allow_always` grant.
Nexa advertises and serves text-file and terminal RPCs. File access uses the
existing workspace/source policy; writes retain checkpoints and change records.
Terminals preserve cwd/environment, bound output, stream their state into tool
cards, and support asynchronous wait, kill and release with process-tree cleanup.
Commands without argv use the host shell, as required by Goose; explicit argv
remains literal. Native tool reports, including file diffs and released terminal
output, are projected without executing their effects again. These client services
do not sandbox the external process's own tools.

Native questions with multiple one-time choices require the exact selected
answer. Generic allow-all policy and reusable tool approval cannot choose an
answer or promote a one-time decision into a permanent native grant.
The same exact-choice flow is available on the phone and projects a selected
answer separately from an allowed/denied tool permission. Waiting for an answer
does not block filesystem requests, terminal output or cancellation.

Text/thinking, native tool lifecycle, context usage snapshots and final message
IDs flow through the existing ordered outbox. A fresh session receives bounded
reference history. Completed sessions are cached by conversation, profile and
launch configuration, with at most four idle sessions and a five-minute idle TTL.
A warm turn sends only new input and changed context. Transcript edits, route
changes, changed cwd, cancellation, process death and uncertain outcomes require
a fresh session; effectful prompts are never retried automatically. Startup and
waiting stages project into a single compact composer status row. Live renderer
reload reads the existing outbox without resending a prompt. User steering is queued until the current native prompt ends.
Stop sends `session/cancel` and tears down the process tree; transport loss and
incomplete/unknown stop reasons retain partial text without emitting success.
Native tool reports must finish before a successful terminal event. Context
usage is not added as billable usage; missing native token/cost data is not
estimated from an API price table.

Context usage retains the native capacity, occupied tokens and provider/model
identity. Compaction can lower occupancy; billing remains cumulative and separate.
Copilot's `session.usage_info`, Codex's thread token snapshot and ACP's `usage_update`
feed this state through live events and durable conversation usage. Switching
models cannot reuse another runtime/model's context capacity. Completed progress
and questions are checkpointed during a turn instead of consuming a lifetime
block limit. Copilot's event queue is drained into a temporary spool so a slow
renderer/database does not lose ephemeral usage, filter or completion events.

AI Usage also records external-runtime consumption in the same invocation ledger
as API calls. Codex cumulative token snapshots and Copilot usage events are
persisted while the turn runs and settled on success, cancellation, or failure.
Repeated snapshots do not add consumption again; model changes retain separate
model attribution. External records aggregate a runtime turn per model, so their
request count is not an inferred count of hidden native model calls.

ACP adapters that supply `session/prompt` response `usage` are supported. Gemini
CLI reports per-prompt totals; Hermes currently reports cumulative session totals,
so Nexa retains a baseline with the pooled ACP session and records only the new
consumption. Counter resets start a new accounting epoch. Missing snapshots keep
the last valid baseline; later catch-up usage includes the intervening consumption
and is explicitly marked as a session interval observed on that turn, not an
exact per-turn measurement. Cache and reasoning details never get added to a
provider's already reported total. Adapters such as pi ACP that only expose
context occupancy retain an unknown usage record. Subscription price and account
quota are not fabricated from these token totals.

Nexa's tools, subagent scheduler, MoA and strict read-only Plan
policy are not exposed by this ACP adapter. The composer hides controls it cannot
honor. Native agent tool/configuration capabilities remain owned by that agent.
Image input requires the runtime's advertised capability. Upstream persisted
session/load, native audio and interactive terminal login are not implemented.
Nexa's explicit checkpoint Resume can continue in a fresh native session after
restart, carrying retained output and reconciliation instructions. It does not
claim exact restoration of an interrupted native tool or replay it automatically.

Protocol references: [Zed external agents](https://zed.dev/docs/ai/external-agents),
[ACP v1](https://agentclientprotocol.com/protocol/v1/initialization),
[Gemini authentication](https://geminicli.com/docs/get-started/authentication/),
[OpenCode ACP](https://opencode.ai/docs/acp/),
[Hermes ACP](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/acp.md).

Malformed historical activity records or event journals are isolated by their
database key. Their original rows remain available for repair, and their IDs
cannot be observed, mutated or reused as valid activities. Unrelated new chats
retain durable activity storage. SQL/storage failures still fail initialization;
the runtime does not silently switch to an in-memory journal.

## Official runtime boundaries

- Copilot uses the pinned SDK/verified CLI in Empty mode, explicitly selects
  custom Nexa tools and denies native permissions. Its official home remains the
  credential owner; Nexa does not copy tokens. User steering is queued until the
  current Copilot operation becomes idle, then sent in the same upstream session.
  Empty mode's forced keychain-disable environment setting is overridden with
  the caller's original setting, so enrollment and execution use the same
  system-keychain or explicitly selected file credential backend.
  Nexa disables SDK tool deferral/search for this route and explicitly registers
  the complete tool inventory; editing tools remain callable above the SDK's
  default 30-tool deferral threshold. The separately labelled Copilot CLI ACP
  route uses native tools and native permissions instead.
- Codex uses a fresh ephemeral app-server thread, no executor environments,
  read-only policy and disabled shell, web, plugins, hooks, agents and automation.
  Effective MCP names and skill paths are inventoried and disabled for that
  thread without changing global config. An inventory error prevents submission.
  This enumeration is not an OS sandbox or an atomic ban on skills created after
  the inventory. The supported CLI must accept the complete execution contract.
  Its stock executor-permission prompt is disabled because it incorrectly labels
  Nexa's host tools as read-only. Nexa supplies the actual host-tool scope and
  approval guidance; the Codex executor's read-only policy remains enforced.
- Codex reserves the `mcp__` dynamic-tool namespace. Nexa sends stable protocol
  aliases and restores the original registered names at dispatch, preserving
  connector identity, permission rules and tool receipts. Runtime warnings remain
  visible in the status row and ordered trace.
- Codex native clock requests receive the actual host time. Native asynchronous
  questions/status are persisted visibly, and are never interpreted as terminal
  answers. Real user replies use `turn/steer`; no suggested option is auto-sent.
  Unknown native requests receive a correlated error and terminate visibly.
- Copilot assembles response chunks by `apiCallId` and `chunkIndex`; empty
  reasoning-boundary chunks count toward completeness but reasoning is not
  included in the answer. Missing or conflicting chunks cannot emit success.
  Completed responses are saved before queued steering is submitted.
  Retry events must match the active native turn. They discard its abandoned
  completed and delta-only drafts before failure recovery and clear the same
  live blocks with canonical snapshots, preserving already-saved responses.
- The final response carries its exact persisted assistant message ID. The
  event outbox commits the closing event, task status and open conversation
  turn together; the ID must belong to that conversation and turn. Existing
  tool traces and already-finalized API turns are preserved.
- Subscription drivers validate the selected model and reasoning level before
  inference. There is no automatic switch to another model or API credential.
- Copilot and Codex parents can delegate to configured API workers through Nexa.
  Use `list_subagent_models` and select `agent_config_id` on each worker; a
  subscription cannot be silently inherited as API credentials. Subscription
  child runtimes, Mixture of Agents and scheduled isolated patch runs remain
  unavailable. Unsupported routes fail before inference.
- Manual `/compact` is unavailable for subscription conversations. Its API
  summarizer cannot consume an official subscription login; the action is hidden
  and direct requests fail before constructing an HTTP provider. The native
  runtime continues to manage context inside each active turn.

## Reconciliation and microphone behavior

Dictation and next-turn orchestration controls remain available while the
current response streams. Changing Nexus, quality or MoA updates the next-turn
selection; it does not mutate the already-running executor. The recording dock
expands above the controls and remains mounted through its exit animation,
respecting reduced-motion preferences.

The heartbeat carries the committed event high-water mark. Receiving a heartbeat
does not postpone the recovery watchdog, so an alive backend cannot indefinitely
hide missing messages. Expanded browser controls are positioned inside the app's
content area. Native webview getters run outside the shared browser mutex, and
trusted native input checks execute on the UI thread to avoid cross-thread waits.

Official legacy Qwen microphone presets migrate once to
`dashscope_realtime_asr` / `qwen3-asr-flash-realtime`. The live WebSocket uses
server VAD, publishes interim text while recording, and finishes with
`session.finish`. Ordered utterance finalization retains earlier sentences and
flushes the last sentence. User composer corrections retain ownership. Custom
HTTP endpoints stay final-only and remain labelled as such; they are not silently
converted into a guessed WebSocket service. After migration, explicitly choosing
a batch preset is preserved across settings saves and reloads.

## Verification

Automated contracts cover native event projection, async question persistence,
tool idempotency/schema failures/budget/cancellation, provider enrollment and
model/reasoning selection, recording composer behavior, browser control bounds
and event reconciliation. Ignored desktop tests named `native_*` explicitly opt
into the user's official login and one read-only tool inference. They assert a
fresh tool nonce reaches the streamed answer, executes once, persists once, and
emits one terminal event through the real forwarder/outbox, and closes the turn
with the exact final assistant ID before delivery. They are not run by ordinary CI.

Additional opt-in native edit tests use the full tool catalog and real read/edit
implementations against one disposable temporary file. Deterministic ACP peers
exercise config replacement, provider/model dependency, Unicode filesystem edits,
terminal polling beyond 512 requests, released output, native choices, cancellation
and context compaction. Passing these contracts is not a claim that every external
CLI version or account has passed live inference; Check connection reports this
boundary explicitly.

Implementation: [subscription drivers](../../apps/desktop/src-tauri/src/agent_runtime),
[account enrollment](../../apps/desktop/src-tauri/src/commands/subscription_accounts.rs),
and [external tool session](../../crates/core/src/agent/external_tools.rs).
The latter owns callback idempotency and the subscription callback budget;
it is distinct from the direct API executor's tool-batch accounting.

Phone chat reuses these desktop drivers. Live capture uses the separate
[Voice and Live](LIVE.md) connection rules; an official subscription login is
not handed to a guessed realtime API endpoint.
