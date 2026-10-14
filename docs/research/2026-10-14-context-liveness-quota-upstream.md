# Runtime context, compaction, quota and liveness: upstream evidence

Research date: 2026-10-14 (Asia/Shanghai). This is primary-source research for implementation and acceptance, not a claim of authenticated provider inference. No credentials were read and no model requests were submitted. Repository source was fetched file by file; no upstream checkout or release asset was downloaded.

## Source revisions

| Project | Inspected immutable revision |
| --- | --- |
| GitHub Copilot SDK, current main | [`508247d16fd9c64f64c439ffac0d9f0216e76005`](https://github.com/github/copilot-sdk/tree/508247d16fd9c64f64c439ffac0d9f0216e76005) |
| Copilot SDK already installed by Nexa, Rust 1.0.11 | Local crate `.cargo_vcs_info.json` reports [`a550258d5c37bd662197536992a23d633bfe5804`](https://github.com/github/copilot-sdk/tree/a550258d5c37bd662197536992a23d633bfe5804), with publication `dirty: true`; local packaged source is the exact compilation authority |
| ACP v1 specification/schema | [`db73d70a2a978e3ffff504f5f4427337f02a1218`](https://github.com/agentclientprotocol/agent-client-protocol/tree/db73d70a2a978e3ffff504f5f4427337f02a1218) |
| OpenAI Codex | [`ca2b076fffbc24ad5fa57258ece8a8277b326138`](https://github.com/openai/codex/tree/ca2b076fffbc24ad5fa57258ece8a8277b326138) |
| Pi (`badlogic/pi-mono`, now redirected to `earendil-works/pi`) | [`3daa20280f6a7c7cbe0935680c9a38fdc3863005`](https://github.com/earendil-works/pi/tree/3daa20280f6a7c7cbe0935680c9a38fdc3863005) |
| Hermes | [`e06df9c2eae6901f4c88f464e0ae6e753c0428b8`](https://github.com/NousResearch/hermes-agent/tree/e06df9c2eae6901f4c88f464e0ae6e753c0428b8) |
| DeepSeek Harness | [`d743267388641bc76f17c45ce8b4c231aed1d32c`](https://github.com/deepseek-ai/deepseek-harness/tree/d743267388641bc76f17c45ce8b4c231aed1d32c) |

DeepSeek's [official Harness page](https://www.deepseek.com/en/harness/) links `deepseek-ai/deepseek-harness`; this name is unambiguous here. The latest stable Copilot SDK release observed was [v1.0.21](https://github.com/github/copilot-sdk/releases/tag/v1.0.21), regenerating types for CLI 1.0.97. Main is ahead of that stable release. A dependency update is not required merely to expose the context tier: Nexa's pinned 1.0.11 already has the fields below.

## Copilot: context is a native tier and a runtime measurement

The packaged Rust `SessionConfig` and `ResumeSessionConfig` expose `context_tier: Option<String>` and `with_context_tier`, using `default` or `long_context`. `SetModelOptions` uses `Option<ContextTier>`. This is discrete upstream configuration, not permission to set an arbitrary token count. BYOK model-limit overrides are a separate provider contract. `InfiniteSessionConfig` separately controls background-compaction and buffer-exhaustion thresholds (defaults 0.80 and 0.95). [Rust types](https://github.com/github/copilot-sdk/blob/a550258d5c37bd662197536992a23d633bfe5804/rust/src/types.rs).

There is no `supported_context_tiers` array on the inspected `Model` type. Positive `billing.tokenPrices.longContext` metadata explicitly describes an available extended-context tier; it is a conservative discovery signal. Its `maxPromptTokens` is an input budget, with deprecated `contextMax` as older metadata. `capabilities.limits.max_context_window_tokens`, `max_prompt_tokens`, and `max_output_tokens` retain snake_case on the wire. Do not confuse those with camelCase billing fields. `SessionContextInfo` reports `modelName`, `totalTokens`, `promptTokenLimit`, `limit`, and `compactionThreshold`; `limit` includes output reserve, while `promptTokenLimit` is the occupancy denominator. [Generated API types](https://github.com/github/copilot-sdk/blob/a550258d5c37bd662197536992a23d633bfe5804/rust/src/generated/api_types.rs).

Migration recommendation: advertise tiers from the exact account/model catalog; default to native behavior; preserve an explicit saved tier per profile/model; revalidate before submission. A model change must not inherit an unavailable long tier. Report requested selection separately from runtime-confirmed measurement. GitHub documents that extended windows consume more AI credits; the control should expose that meaningful tradeoff. [GitHub supported models and extended capabilities](https://docs.github.com/en/copilot/reference/ai-models/supported-models#models-with-extended-capabilities).

`session.compaction_start` does not mean foreground generation stopped: compaction can run in the background before the blocking threshold. `session.usage_info` is a replacement occupancy snapshot (`currentTokens`, `tokenLimit`), not billable deltas. `assistant.usage.model` identifies the executing model. `assistant.streaming_delta.totalResponseSizeBytes` supplies network progress without visible text. Envelope `agentId` distinguishes child activity. `session.idle` and deltas are ephemeral; startup subscription races cannot be repaired by replaying only persisted messages. [Streaming event contract](https://github.com/github/copilot-sdk/blob/508247d16fd9c64f64c439ffac0d9f0216e76005/docs/features/streaming-events.md).

Recommendation: persist a compaction activity state independently of foreground streaming; allow both to be true. Show background compression as background work, and clear it on its actual completion/failure or terminal session cleanup. A UI timeout must not be the mechanism that reconciles incompatible states. Root context must ignore a child's usage/model events. A real lower post-compaction snapshot replaces the old value.

## ACP: negotiate compaction instead of parsing ordinary text

Current v1 requires `clientCapabilities.session.compaction: {}` before an agent may send `compaction_update` or `compaction_summary_chunk`. Each update has opaque `compactionId` and required status; known states are `in_progress`, `completed`, `failed`, `cancelled`. Patches preserve position; omitted fields retain values, `null` clears optional fields, summary chunks append, and `summary` replaces. Compaction does not itself replace usage or authorize discarding chat history. `usage_update.used` and `.size` are required replacement token counts. Notice capability is separate; a notice with severity `error` is not a failed prompt. [ACP prompt-turn source](https://github.com/agentclientprotocol/agent-client-protocol/blob/db73d70a2a978e3ffff504f5f4427337f02a1218/docs/protocol/v1/prompt-turn.mdx).

ACP `configOptions` and `config_option_update` contain a complete replacement configuration state. The agent can change the model after fallback; the composer selection is not proof of current execution identity. Apply provider/mode dependencies first, then model, and consume the replacement options. [Session configuration](https://agentclientprotocol.com/protocol/v1/session-config-options).

ACP `session/cancel` cancels the entire prompt turn. It is not a scoped computer-use stop API. The client settles pending permission requests as cancelled, continues accepting terminal tool updates, and waits for the original prompt response's cancelled stop reason. Therefore, Nexa's computer overlay must revoke its own computer-control capability, not forward session cancellation. [ACP cancellation](https://agentclientprotocol.com/protocol/v1/prompt-turn#cancellation).

## Long tasks: separate transport, work and UI clocks

| Evidence | Concrete application in Nexa |
| --- | --- |
| Codex `ErrorNotification` contains `willRetry`, `threadId`, `turnId`; `ModelReroutedNotification` contains `fromModel`/`toModel`. [Error schema](https://github.com/openai/codex/blob/ca2b076fffbc24ad5fa57258ece8a8277b326138/codex-rs/app-server-protocol/schema/typescript/v2/ErrorNotification.ts), [reroute schema](https://github.com/openai/codex/blob/ca2b076fffbc24ad5fa57258ece8a8277b326138/codex-rs/app-server-protocol/schema/typescript/v2/ModelReroutedNotification.ts) | A retryable notification is not terminal failure. Keep provider-owned retries under one owner; update actual model identity on a matching active turn. |
| Codex `ThreadTokenUsage` separates `total`, `last`, and nullable `modelContextWindow`. [Schema](https://github.com/openai/codex/blob/ca2b076fffbc24ad5fa57258ece8a8277b326138/codex-rs/app-server-protocol/schema/typescript/v2/ThreadTokenUsage.ts) | Cumulative billing must not become current context occupancy. Unknown native capacity must remain unknown. |
| Pi separates `agent_end(willRetry)` from `agent_settled`; compaction and retry have explicit events and abort controllers. `getContextUsage` declines stale pre-compaction usage until another response. Overflow is a compaction concern, not generic transient retry. [Agent session](https://github.com/earendil-works/pi/blob/3daa20280f6a7c7cbe0935680c9a38fdc3863005/packages/coding-agent/src/core/agent-session.ts) | Cleanup at authoritative settlement, not at the first message end. Backoff stays cancelable; retry status ends on cancellation; a context reduction must not reuse old usage. |
| Hermes maintains request-local last-real-chunk time, distinguishes first-chunk from post-chunk silence, and sends gateway activity independently. Wait notices clear when real chunks resume. [Stream monitor](https://github.com/NousResearch/hermes-agent/blob/e06df9c2eae6901f4c88f464e0ae6e753c0428b8/agent/chat_completion_stream_monitor.py) | Track upstream byte activity, semantic progress and UI heartbeat separately. A synthetic Nexa heartbeat must not renew a provider stall deadline. Real reasoning, tool progress and owned local loading can explain silence without fabricated answer text. |
| DeepSeek compaction records start, replacement and end with one owner; it retries overflow recovery only after the context surface advances. Its compaction backend is singular per context. [Compaction](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/docs/subsystems/compaction.md) | Preserve one compaction owner per runtime. Do not add another summarizer alongside ACP/Copilot's native one. Orphaned state after restart must reconcile rather than remain “compressing” forever. |
| DeepSeek has explicit turn cancellation, one signal throughout an owned turn, and a fresh signal for the next turn; cooperative work is joined before reporting idle. [Cancellation design](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/.agents/notes/archived/architecture/2026-07-16-explicit-turn-cancellation.md) | A timed-out read may be abandoned safely; an uncertain click/write may not be silently repeated. Generation-bound cancellation prevents a stale overlay action from cancelling a later operation. |

The current Nexa Copilot adapter already disables the SDK's default 60-second `send_and_wait` deadline. Merely increasing that timeout again would not address a stalled event projection or tool/cancellation ownership bug. Audit the actual receipt/read/dispatch/emit/ack chain, and attach bounded diagnostics to the stage that stopped progressing.

## Subscription quota: finish the primary read independently

Codex `account/rateLimits/read` is quota; prefer `rateLimitsByLimitId` to the legacy bucket. `usedPercent` means consumed, and `resetsAt` is Unix seconds. `account/usage/read` is a separate optional token-activity read (lifetime totals, streaks and daily buckets); it must not delay or erase successful quota. [Official app-server documentation](https://learn.chatgpt.com/docs/app-server).

Copilot `account.getQuota` returns `quotaSnapshots`, keyed by quota type, including remaining percentage, entitlement, reset and unlimited status. Use the runtime's auth; do not derive account balance from chat tokens. [Copilot quota API types](https://github.com/github/copilot-sdk/blob/a550258d5c37bd662197536992a23d633bfe5804/rust/src/generated/api_types.rs).

Hermes caches by profile home, provider, account identity and base URL, throttles failed attempts, releases inflight ownership, and marks stale data unknown. A newer request must not lose to an older response. [Account usage cache](https://github.com/NousResearch/hermes-agent/blob/e06df9c2eae6901f4c88f464e0ae6e753c0428b8/agent/account_usage_cache.py).

Recommendation: every read reaches ready, unavailable, auth-required or error within a bounded deadline, including process startup and cleanup. Optional enrichment cannot hold the loading state. Closing/unmounting and profile changes invalidate request ownership; timeout releases inflight state so Refresh works. Preserve an explicit stale timestamp if retaining last-known data; do not display it as freshly measured. Unsupported ACP quota is unavailable, not perpetually loading.

## Computer use: stop the control session, retain the conversation

DeepSeek's computer-use service admits only one provider, holds registration until owned work stops, and explicitly notes that cancellation cannot undo already-delivered desktop input. [Computer-use lifetime](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/docs/subsystems/computer-use.md). Hermes latches cancellation so work registering after a stop is immediately interrupted. [Interrupt scope](https://github.com/NousResearch/hermes-agent/blob/e06df9c2eae6901f4c88f464e0ae6e753c0428b8/agent/interrupt_scope.py).

Nexa recommendation: the overlay owns a computer-use session/generation. Stop cancels current and queued computer actions, rejects subsequent actions under that generation, releases held inputs, and returns a structured stopped receipt to the agent; the chat can explain or continue other work. Do not conflate hiding the overlay with revoking control. Keep the existing observe/act/verify fence. Use the bundled Nexa logo; short opacity/translation/scale entrance and a restrained logo accent are UI proposals, not upstream requirements. Respect reduced motion and provide a keyboard-accessible Stop label.

## Provider/model audit deltas and discovery authority

The inspected text preset contains 23 endpoint presets (including two local servers, separate Qwen regions/plans, and retired Yi). Existing non-chat catalogs cover embeddings, images, video, speech and decisions. Presets are suggestions plus lifecycle records, not proof of entitlement. A native vendor retirement cannot automatically retire the same family on an aggregator. Keep removed IDs as nonselectable tombstones where needed to explain existing settings.

### Confirmed changes since the previous local audit

- **Google:** add exact `gemini-4-argon`. Unified input/thinking/output context is 1,048,576. Direct REST caps/defaults are 32,768 nonstreaming and 65,536 streaming; current SDKs continue automatically. Thinking is low/medium/high, default medium; it cannot be disabled. Larger output requires continuation or background execution, so an unchanged single-request adapter must retain its route-specific cap. [GenerateContent model/migration guide](https://ai.google.dev/gemini-api/docs/generate-content/latest-model?hl=en), [thinking contract](https://ai.google.dev/gemini-api/docs/generate-content/thinking?hl=en).
- **OpenAI:** the October 14 announcement deprecates GPT-4o, GPT-4o mini, GPT-4.1 and GPT-4.1 mini snapshots for January 14, 2027 shutdown. Mark affected native selections deprecated, not removed. Existing October 1 GPT-5.3-Codex/GPT-5.4-Nano retirement remains April 1, 2027. [Official deprecations](https://developers.openai.com/api/docs/deprecations).
- **OpenRouter:** anonymous `/models` returned 463 rows. Missing ordinary text-output, nonbatch IDs in the local fallback: `google/gemini-4-argon`, `liquid/lfm-2.5-24b-a2b`, `~unbiased/pareto-latest`, `unbiased/pareto-26.9`, `scaledown/summarize`, `scaledown/extract`. The last two expose only `max_tokens`, not tools, and must not be promoted to tool-capable agents. [Public text catalog](https://openrouter.ai/api/v1/models).
- **OpenRouter removals:** `apodex/apodex-1.1-mini:free`, `inclusionai/ling-3.0-flash-fin`, and `inclusionai/ling-3.0-flash-sante` were absent from `/models` and their endpoint APIs returned empty endpoint arrays. They are not currently callable routes. In contrast, the missing older Google preview alias still resolved seven endpoints; keep it as an alias. [Apodex](https://openrouter.ai/api/v1/models/apodex/apodex-1.1-mini:free/endpoints), [Fin](https://openrouter.ai/api/v1/models/inclusionai/ling-3.0-flash-fin/endpoints), [Sante](https://openrouter.ai/api/v1/models/inclusionai/ling-3.0-flash-sante/endpoints), [Google alias](https://openrouter.ai/api/v1/models/google/gemini-2.5-pro-preview-05-06/endpoints).
- **OpenRouter modality coverage:** image catalog returned 57 rows; eight unbundled entries were styles/vector variants, requiring their distinct reference/output contract. Embeddings returned 33 rows; catalog presence alone supplies neither dimensional defaults nor verified dimensional override support. [Images](https://openrouter.ai/api/v1/images/models), [embeddings](https://openrouter.ai/api/v1/embeddings/models).

### Current endpoint coverage and retention checks

| Existing endpoint | Source-backed result / discovery requirement |
| --- | --- |
| OpenAI | Current GPT-6 family already exists locally. Apply dated deprecations; use the configured account's catalog for entitlement. [Model catalog](https://developers.openai.com/api/docs/models/all) |
| Anthropic | Current Models API supports paginated `lifecycle=active|deprecated|retired` filtering (default active+deprecated), and explicit workspace selection. Consume retirement metadata, not absence from one page. Sonnet 4.5 is deprecated; prior Opus 4.1/4/Haiku 3 retirements remain. [Models API](https://platform.claude.com/docs/en/api/models/list), [retirements](https://platform.claude.com/docs/en/about-claude/model-deprecations) |
| Google | Gemini 2.5 Pro/Flash/Lite remain served for prior users and are explicitly not deprecated. Gemini 3.7 Flash and 3.5 Flash are functioning redirects, not shut-down endpoints. [Lifecycle](https://ai.google.dev/gemini-api/docs/deprecations) |
| DeepSeek | Current direct IDs remain `deepseek-flash` and `deepseek-v4-pro`; old V4 Flash IDs route to V4.1. The direct docs fetch timed out in this run; indexed first-party pricing and changelog agreed. Do not infer Pro retirement from an obsolete notice. [Pricing](https://api-docs.deepseek.com/quick_start/pricing/), [updates](https://api-docs.deepseek.com/updates/) |
| xAI | Current directory retains Grok 4.7 and endpoint-specific image models. Native and OpenRouter IDs/limits stay separate. [Directory](https://docs.x.ai/developers/models) |
| Xiaomi MiMo | Existing V2.6 family matches docs. V2.5 names retire October 21; October 14 rerouting is documented with inconsistent 10:00/18:00 times, so do not encode a guessed cutoff. [Deprecation notice](https://mimo.mi.com/docs/en-US/updates/deprecate) |
| Meta | Existing `muse-wonder-1.0` and `muse-spark-1.3` match current docs. Wonder Full is subscription-only, not a Model API ID. `/v1/models` is team-scoped. Contributor IDs change data-use terms and must never become silent replacements. [Models](https://dev.meta.ai/docs/models) |
| MiniMax | M3.1 Flash Preview remains M Plan/MiniMax Code gated. M2.5/M2.7 paid API retire November 18; plans reroute October 19. Preserve deprecated status until the relevant route's shutdown. [Models](https://platform.minimax.io/docs/guides/models-intro) |
| Mistral | Existing Large 4 and hosted GLM 5.3 appear in current directory; deprecated is distinct from retired. [Directory](https://docs.mistral.ai/models) |
| Zhipu CN / Z.ai international | Both official published pages independently confirm `glm-5.3-flash`/`glm-5.3-flashx`, 1M context, 128K output, enabled-only thinking and FlashX exclusion from Coding Plan. The initial CN text-fetch timeout was resolved by reading its published HTML and Markdown. [International](https://docs.z.ai/guides/vlm/glm-5.3-flash), [CN](https://docs.bigmodel.cn/cn/guide/models/vlm/glm-5.3-flash) |
| Moonshot | Current exact four IDs match local active rows; K2.5/moonshot-v1 discontinued August 31, K2 series May 25. [Models](https://platform.kimi.ai/docs/models) |
| SiliconFlow | Current hosted-model center fetched; regional catalog and provider-prefixed IDs remain authoritative. No inferred direct-vendor migration. [Model center](https://www.siliconflow.cn/models) |
| Qwen Token Plan CN/global | Current plan tables match the existing text families and `auto`. CN qwen3.8-max-preview calls redirect to qwen3.8-max. These are plan hosts/credentials, not pay-as-you-go entitlements. [CN](https://help.aliyun.com/zh/model-studio/token-plan-personal-overview), [global](https://www.alibabacloud.com/help/en/model-studio/token-plan-personal-overview) |
| Qwen Cloud international / Alibaba CN | Latest listed image addition `qwen-image-2.1-turbo` (October 9) is already present locally. Preserve exact hosted namespaces (`ZHIPU/`, `kimi/`, `stepfun/`, etc.) and regions; a provider's public table is not a catalog of enabled account models. [CN changes](https://help.aliyun.com/zh/model-studio/newly-released-models), [international changes](https://www.alibabacloud.com/help/en/model-studio/newly-released-models) |
| Doubao/Ark | Official model/release/retirement tables were verified in a temporary hidden browser after text-only fetching returned a shell. Newest September rows match the local preset; additional supported IDs and dated retirement corrections are below. [Model table](https://docs.volcengine.com/docs/ark/model-list?lang=en), [release table](https://docs.volcengine.com/docs/ark/model-release-announcement?lang=zh) |
| Yi | Current published homepage application code explicitly confirms API/model services stopped September 3, 2026 at 24:00; existing removed state is correct. Announcement August 3, refunds through December 3. This is positive source evidence, not an inference from an empty fetch. [Platform](https://platform.lingyiwanwu.com/), [published page code](https://platform.lingyiwanwu.com/_next/static/chunks/app/page-276ce40cfdd8932a.js) |
| Baichuan | Current pricing page continues the M3/M2 API family. [Pricing](https://platform.baichuan-ai.com/prices) |
| Ollama / LM Studio | Discover installed/visible models from `/api/tags` and `/v1/models`; do not install or delete user models as catalog maintenance. LM Studio JIT may list downloaded but unloaded models. [Ollama](https://docs.ollama.com/api/tags), [LM Studio](https://lmstudio.ai/docs/developer/openai-compat/models) |

The repository's `scripts/model-catalog-audit.mjs` covers all seven catalog surfaces structurally, but `--live-public` checks a selected OpenRouter required-ID set, not all providers. A green run therefore cannot be described as authenticated exhaustive vendor compatibility. Every newly selected model still needs the existing endpoint-scoped request/response and reasoning profile checks; modality-specific transports need their own acceptance.

### Follow-up: exact Ark/Zhipu/Yi evidence

The official Ark model table contains these supported tool-capable rows absent from the inspected `doubao` preset. They are catch-up omissions, not claims that these models were released today. Units below retain the page's `k` notation; Nexa's existing Ark catalog uses decimal token conversion.

| Exact Ark ID | Context / max input / max answer | Other declared capability |
| --- | --- | --- |
| `doubao-seed-2-1-pro-260628` | 256k / 256k / 256k | Multimodal, tools, structured output |
| `doubao-seed-2-0-mini-260428` | 256k / 224k / 128k | Multimodal, tools, structured output |
| `doubao-seed-character-260628` | 128k / 96k / 32k | Multimodal, tools, structured output |
| `glm-5-2-260617` | 1024k / 1024k / 128k | Text and tools; no structured-output claim in this row |
| `deepseek-v4-pro-ga-260813` | 1024k / 1024k / 384k | Text, tools, structured output |
| `deepseek-v4-flash-ga-260731` | 1024k / 1024k / 384k | Text, tools, structured output |

Source: [rendered official Ark model table](https://docs.volcengine.com/docs/ark/model-list?lang=en). The June Pro ID is also an example in the [official Ark Python SDK](https://github.com/volcengine/ark-runtime-python/blob/88f041f568d2f578e4ce0ce1f622f3b84b9faa18/README.md); an SDK example alone is not a latest-model claim. Do not transfer any of these IDs to the direct DeepSeek/Z.ai endpoints or the separate Ark Agent Plan.

Ark's October 9 retirement notice places the tenth batch in EOM on September 24 and EOS on November 24 at 14:00 UTC+8, except `doubao-seed-2-0-lite-260215` and `doubao-seed-2-0-mini-260215` end October 22 at 14:00. Existing `doubao-seed-2-0-pro-260215` remains deprecated; the notice now recommends `doubao-seed-2-1-lite-260915` (local metadata still recommends Pro). Ninth-batch September 21 retirement confirms the local removed Seed 1.6/code-preview/Flash rows. Embedding EOM does not stop existing endpoints; endpoint migration and model retirement are distinct. [Retirement notice](https://docs.volcengine.com/docs/ark/model-deprecation-notice?lang=zh).

The Zhipu [model overview](https://docs.bigmodel.cn/cn/guide/start/model-overview.md) still lists GLM-5.2/5.1/5/5-Turbo/4.7/4.6 and older paid/free families, so there is no support for mass removal based on age. Additional unbundled examples include GLM-4.5-Air/AirX, GLM-4.7-Flash, GLM-4.6V-Flash and GLM-4.1V-Thinking-Flash; exact endpoint IDs/request profiles require their individual pages before catalog promotion. The [GLM-5.3 migration guide](https://docs.bigmodel.cn/cn/guide/start/migrate-to-glm-new.md) confirms thinking is forced on and tool streaming needs `stream: true` plus `tool_stream: true`; recommending migration is not a retirement notice.

Yi's above published page bundle was 3,695 characters; SHA-256 of its UTF-8 response text was `a91eec4a930189ce82d8e5c2d647abbf7d4272bb61b0f341a0de8aeb6a57b704`. The public notice has no account-dependent state. No login, refund action or user-account query was performed.

### Follow-up: Ark reasoning settings are endpoint-specific

The native Chat/Responses [deep-thinking guide](https://docs.volcengine.com/docs/ark/deep-thinking) was verified from its published [document API](https://docs.volcengine.com/api/doc/getDocDetail?LibraryCode=ark&DocumentCode=deep-thinking), updated October 9. All six rows accept enabled/disabled thinking and default to enabled. Their effective effort levels differ:

| Exact native Ark IDs | Default | Effective settings and compatibility mappings |
| --- | --- | --- |
| `doubao-seed-2-1-pro-260628` | high | minimal/low/medium/high; none maps to minimal/off, xhigh/max to high |
| `doubao-seed-2-0-mini-260428`, `doubao-seed-character-260628` | medium | minimal/low/medium/high; same mappings |
| `glm-5-2-260617` | high | off/high/max; none/minimal disable thinking, low/medium map to high, xhigh to max |
| `deepseek-v4-pro-ga-260813`, `deepseek-v4-flash-ga-260731` | high | off/low/high/max; none/minimal disable thinking, medium maps to low, xhigh to high |

Seed 2.1 Pro uses thinking-summary ciphertext (`encrypted_content`) for replay; Mini, Character, GLM and the two DeepSeek rows use ordinary `reasoning_content`. Ark's separate [Managed Agent parameter table](https://docs.volcengine.com/docs/ark/model-parameter-support) gives GLM 5.2 a max default; that must not override native Chat/Responses. Direct DeepSeek's minimal mapping also differs from Ark's. Preserve these route distinctions.

### Follow-up: exact Gemini 4 REST continuation contract

The [GenerateContent long-decode guide](https://ai.google.dev/gemini-api/docs/generate-content/long-decode?hl=en) specifies:

- Read `candidates[i].continuationToken` with `finishReason: "CONTINUATION"`. Resubmit the original request with **top-level** `continuationToken`; preserve `contents`, system instruction, tools, and generation configuration. Do not append partial generated output to the input. `maxOutputTokens` is the same total across hops.
- `MAX_TOKENS` means the explicit total budget is exhausted. Its token can resume only if the caller deliberately raises that budget; otherwise stop. Empty text plus a continuation token may represent a hop spent thinking.
- Streaming checkpoints arrive every 5,000 tokens after 32,768. Hold uncheckpointed chunks pending; commit at a checkpoint, and on disconnect discard the later pending tail before resuming. Keep the final usage snapshot per hop; billing adds hops, whereas context uses the original prompt plus generated tokens. `initialPromptTokenCount` is SDK-derived, not a raw REST field.
- Set HTTP read timeout to 20 minutes **per hop**. The server ends nonstreaming connections after 10 minutes and streams after 20. This is not a total task deadline. Neither continuation-token nor thought-signature expiry is documented; the stated 15–20-minute TTL is implicit-cache expiry.

The page's prose says an initial nonstreaming REST request above 32,768 without a continuation token returns HTTP 400, but its curl example uses 262,144. Treat this as a documentation inconsistency: retain the 32,768 raw nonstream cap until verified, and implement long decode on the explicitly documented streaming path. In [migration guidance](https://ai.google.dev/gemini-api/docs/generate-content/latest-model?hl=en), remove ignored temperature/top-p/top-k/candidate-count controls, reject explicit caching, and preserve thought signatures and part boundaries.

### Follow-up: what Harness discovery actually provides

The immutable Harness [LLM contract](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/packages/llm/llm/src/types.ts) defines `LlmModelDiscoveryRequest { provider?, baseURL?, api?, apiKey? }`; the credential is operation-only. `LlmModelDiscoveryOperation` adds `AbortSignal`. Results carry ID, optional name/context/output capacity/input modalities. Resolved model info separately supplies reasoning and update capabilities. A payload-free `llm/adapters-updated` event prompts callers to reread provider/model/configurable-provider lists.

Its actual [pi-ai discovery implementation](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/packages/llm/llm-pi-ai/src/discovery.ts) prefers the installed registry without network access. Unknown routes can list OpenAI Completions/Responses or Anthropic Messages; Azure and OAuth Codex are explicitly unsupported. Gateway replies may use a `data` array or enriched `models` map. Positive integer limits are retained; malformed rows are skipped; missing metadata stays absent. Abort covers fetch and body reading, with a 4 MiB response limit.

This is candidate discovery for a configuration draft, **not automatic catalog refresh**. Anthropic reads one page of 1,000 without following `has_more`; it is not an exhaustive-pagination reference. Borrow the cancellation/provenance/structured-result boundary, but keep Nexa's existing catalog owner and implement provider-native pagination and lifecycle metadata.

### Source-backed discovery gap across providers

The implementation snapshot before this remediation has a common information-loss seam:

1. `crates/core/src/llm/mod.rs` exposes `LlmProvider::list_models() -> Result<Vec<String>, CoreError>`.
2. `crates/core/src/llm/anthropic.rs:2518` returns built-in `preset_model_ids` without calling `/v1/models`. `commands/conversation.rs` and `commands/model_choices.rs` then mark that successful return as live discovery. New Anthropic models, workspace entitlement and explicit retirement metadata therefore cannot be refreshed.
3. `crates/core/src/llm/openai.rs:3157` reads model responses and reduces them to IDs. Google's `list_models` correctly paginates and filters `generateContent`, but also reduces metadata to names.
4. `crates/model-catalog/src/discovery.rs::DiscoveredModel` contains only ID, endpoint, region and optional display name. `core/src/provider_catalog.rs:706` constructs these from strings. The existing shared catalog is already the correct owner; adding a parallel discovery cache would duplicate that responsibility.

Upstream publishes metadata that the ID-only path cannot preserve:

| Upstream discovery contract | Fields to retain through the existing catalog seam |
| --- | --- |
| [Anthropic Models API](https://platform.claude.com/docs/en/api/models/list) | `lifecycle`, `deprecated_at`, `retires_at`, `max_input_tokens`, `max_tokens`, capability flags and effort levels; follow `has_more`/`last_id` pagination and the configured workspace. Do not copy the schema's illustrative zero limits or mismatched example line. |
| [DeepSeek Models API](https://api-docs.deepseek.com/api/list-models/) | `name`, `context_window`, `max_output_tokens`, input/output modalities, `effort.supported_levels`, `effort.default_level`, and per-protocol `api_capabilities`. `context_window` includes input and output. |
| [Google Models API](https://ai.google.dev/api/models) | `displayName`, `inputTokenLimit`, `outputTokenLimit`, `supportedGenerationMethods`, `thinking`; preserve pagination and do not infer that input/output maxima add to a unified runtime budget. |
| [OpenRouter public model catalog](https://openrouter.ai/api/v1/models) | `context_length`, `top_provider.max_completion_tokens`, `architecture`, and `supported_parameters`; do not turn image-only or batch rows into streaming tool models. |

Recommended implementation: add structured discovery to the existing provider trait/catalog path, retaining the ID convenience method only as a projection if callers need it. Map only documented metadata at recognized protocol/endpoint boundaries. Missing fields stay unknown; unsupported-listing and offline fallback must carry honest provenance. Explicit endpoint-scoped retirement wins over stale static selection; absence from one incomplete account list is neither a global tombstone nor evidence of retirement. Test malformed/partial pagination, new model discovery, stale static capabilities, native retirement, custom endpoints, and a successful inference whose model is absent from a provider listing. Read-only discovery must not run inference merely to enumerate the catalog.

## Migration and acceptance checklist

1. One runtime measurement contract: bind usage/capacity/model/tier/source to conversation and turn; a composer change during a running turn must not change that turn's denominator. Late prior-turn events must not overwrite a new turn.
2. Copilot: default and advertised long tier serialize correctly; unavailable tier fails before prompt; actual native capacity wins; root text can continue while background compression appears; completion/failure clears the compression state.
3. ACP: capability negotiation, full config replacement, typed compaction IDs/patches/summary chunks, unknown status preservation, and ordinary-text fallback. Summary and notice must never masquerade as the final answer or terminate the prompt.
4. Quota: stalled startup, unsupported RPC, successful quota plus hanging optional statistics, late profile response, rapid close/reopen, timeout then successful Refresh, unlimited and missing percentages. All cases leave loading and release inflight ownership.
5. Long task: delayed first token, reasoning-only progress, byte-only progress, long tool execution, real network stall, provider retry, event-buffer lag, renderer reload, process exit and resumed turn. Check receipt order and at-most-once tool effects, not only that a spinner eventually disappears.
6. Computer stop: current input ends, held inputs release, queued/stale-generation actions reject, overlay hides after settlement, chat remains usable, next authorized control session receives a fresh generation. Verify keyboard focus, reduced motion and visible animated entrance on the actual overlay.
7. Model refresh: compare exact endpoint/account/region and modality; remove only established unavailable routes from selectable choices; retain tombstones and existing user selections for explanation, without silently selecting a replacement.
8. Release acceptance follows the user's scope: green required checks and final review with no confirmed P1 remaining; publish the intended tag and non-draft release with expected assets. Download verification is explicitly not requested. Rebuildable caches may be removed only after active builds and retained evidence are accounted for.
