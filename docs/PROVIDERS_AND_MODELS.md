# Models and providers

Nexa separates a saved connection, a selected model, and the capabilities of the
exact endpoint behind that connection. Model names alone do not authorize a
protocol, context limit, reasoning control, or credential route.

## Choose a connection

| Connection | Setup and execution |
| --- | --- |
| API provider | Add a provider in Settings, configure its base URL and credentials, save it, then select an available model in Chat |
| Local model service | Configure the supported local endpoint and run that service on the computer; local inference still requires the service and model files |
| Subscription agent | Add GitHub Copilot or ChatGPT / Codex, sign in through its official runtime, and save the provider; see [Subscription agents](SUBSCRIPTION_AGENTS.md) |
| Installed ACP agent | Choose Gemini CLI, OpenCode or Hermes under External agents, set its executable/working directory and check its native model catalog; see [ACP agents](SUBSCRIPTION_AGENTS.md#installed-acp-agents) |

Use the model picker and refresh/discovery actions to inspect current choices.
If a custom endpoint does not support discovery, use its documented model ID.
A saved model choice is not proof that the endpoint will accept a request.
Verify a simple conversation before depending on that connection for a workflow.

API keys and subscription logins are separate credential paths. A subscription
does not become an API key, and selecting an API-only feature does not silently
switch to another connection. Model availability, entitlement, cost, and service
limits remain properties of the configured account and endpoint.

## Capability resolution

The shared [provider presets](../shared/provider-presets.json),
[core provider catalog](../crates/core/src/provider_catalog.rs), and
[capability registry](../crates/core/src/capability_registry) define known
endpoint/model facts. Runtime discovery and explicit configuration supplement
that catalog. The frontend projects the resolved choices instead of maintaining
an independent model inventory.

- Chat, tool use, images, audio, video, embeddings, and speech are distinct
  capabilities. Selecting a model for one does not enable all the others.
- Reasoning effort and thinking budgets appear only where the selected route
  supports them. Nexa orchestration profiles such as Code Ultra are not provider
  reasoning values.
- Unknown custom endpoints remain provider-managed where their capacity or
  protocol cannot be established. Nexa must not guess a public-provider limit
  from an alias or recognizable model name.
- Credentials stay scoped to their configured provider/endpoint. Model discovery
  is not permission to forward credentials to another host.
- Image and speech connections keep their own configuration and capabilities;
  changing one provider must not copy another provider's keys or model settings.

For output continuation, accepted-route replay, context management, and explicit
worker budgets, see [Orchestration runtime](ORCHESTRATION_RUNTIME.md).

## Kimi tool calling

Kimi's OpenAI-compatible envelope does not imply support for arbitrary JSON
Schema. Moonshot uses a restricted dialect (MFJS). For example, a tool with
`type` beside `anyOf` can cause the entire request to fail before the model
produces output, even when that tool is not used in the current turn.

Nexa projects tool parameters at the Chat Completions serialization boundary
for Moonshot's public endpoints and known Alibaba Kimi routes. This covers
streaming and non-streaming requests, built-in tools, and MCP tools. Root object
alternatives expose their fields without making every alternative mandatory;
nested unions, primitive enums, nullable types, and array items use the supported
vocabulary. Local references are expanded with depth and work limits. Recursive
or unresolved references remain unconstrained in the wire description; remote
schema references are never fetched.

The projection is deliberately more permissive where MFJS cannot express the
original contract. Conditional/exclusive requirements and tuple constraints
remain model guidance. The original registered schema and tool execution
validation are unchanged; MCP servers remain responsible for their own full
validation. This is not a general JSON Schema compiler or a guarantee that every
external schema fits the service's size and depth limits.

Verified route distinctions on October 2, 2026:

| Service | Model examples | Thinking contract |
| --- | --- | --- |
| Moonshot public API (`api.moonshot.ai` / `api.moonshot.cn`) | `kimi-k3`, `kimi-k2.6` | Native Moonshot controls, selected per model |
| Alibaba-hosted Kimi | `kimi-k3`, `kimi-k2.6` | `enable_thinking`; hosted K3 is always on and has no numeric thinking budget |
| Moonshot direct supply through Alibaba | `kimi/kimi-k3`, `kimi/kimi-k2.6` | Direct-supply controls; K3 uses the documented `max` effort |
| Alibaba Coding Plan (`coding.dashscope.aliyuncs.com` / `coding-intl.dashscope.aliyuncs.com`) | `kimi-k2.5` | `enable_thinking` and reasoning replay, without borrowing pay-as-you-go budget controls |

Only documented HTTPS base paths receive these adaptations. Other Alibaba
model families, OpenRouter, and private proxy endpoints retain their own schema
contracts. Saved provider credentials, model choices, and retirement rules are
not changed by schema projection. API acceptance for a particular account still
requires a live check using that account's endpoint and entitlement.

Sources: [Moonshot MFJS](https://github.com/MoonshotAI/walle/blob/main/docs/mfjs-spec.md),
[Alibaba-hosted Kimi](https://help.aliyun.com/zh/model-studio/kimi-api),
[Moonshot direct supply](https://help.aliyun.com/zh/model-studio/kimi-api-by-moonshot-ai),
and [Coding Plan thinking controls](https://help.aliyun.com/zh/model-studio/coding-plan-faq).

## Claude Sonnet 5.5

Verified on September 29, 2026: Anthropic `claude-sonnet-5-5` and OpenRouter
`anthropic/claude-sonnet-5.5` have a 1M context window, a 128K output ceiling,
and five effort levels: low, medium, high, xhigh, max (API default: high).
OpenRouter's `~anthropic/claude-sonnet-latest` currently resolves to 5.5.
These catalog facts do not assert availability for a particular credential.

The native API uses adaptive thinking. Turning off **up-front thinking** selects
`between_tools`, which still returns signed progress between tool calls; it is
not a fully disabled thinking mode. Nexa uses high or a supported lower effort
for this mode and does not send removed manual-budget fields. OpenRouter exposes
always-on reasoning through its normalized effort contract. Subscription and ACP
agents continue to obtain model availability from their own runtimes.

Native assistant blocks, including empty signed thinking, retain their original
order in backend replay state. Adaptive requests opt into dropping thinking
invalidated by a changed prompt. For `between_tools`, which does not accept that
option, Nexa checks saved prompt fingerprints and removes affected thinking from
the first changed prefix onward, preserving text and completed tool exchanges.
The fingerprints are computed in a linear pass and exclude only protocol cache
hints. Sensitive interaction turns keep the existing non-replayable persistence
boundary. OpenRouter reasoning details remain gateway-native.

Sources: [Anthropic overview](https://platform.claude.com/docs/en/models/sonnet-5-5/overview),
[migration guide](https://platform.claude.com/docs/en/models/sonnet-5-5/migration-guide),
and [OpenRouter model](https://openrouter.ai/anthropic/claude-sonnet-5.5).

## Structured decisions with Jev

Settings → AI Providers → Structured decisions configures the optional
`evaluate_decisions` tool. It accepts supplied state and typed Choice, Score,
or Noul questions, returning probabilities and positional answers. Scores are
weighted positions on the supplied scale and may be fractional. Results are
advisory; they do not approve tools or satisfy completion gates by themselves.

The [separate System One catalog](../shared/system-one-provider-presets.json)
includes TypeSafe, OpenRouter, SiliconFlow China/international, and Alibaba
Model Studio Beijing/Singapore. Credentials are encrypted and sent only to the
selected regional endpoint. The tool cannot override a configured account,
endpoint, or model. Source text is projected through the current privacy policy;
revocation or cancellation stops the request. Question and option identities
remain stable even when custom redaction changes their business labels.

TypeSafe's direct pinned model is `jev-1.13.0`; OpenRouter uses
`typesafe/jev-1.13` or the bare `jev-latest` alias through `/api/v1/systemone`.
The separately selectable OpenRouter chat model `typesafe/jev-router` chooses
the serving chat model and effort remotely. It is not the decision endpoint,
and Nexa does not select it automatically. See the
[TypeSafe API](https://docs.typesafe.ai/api),
[OpenRouter System One contract](https://openrouter.ai/docs/guides/community/typesafe-sdk),
and [Jev Router contract](https://openrouter.ai/docs/guides/routing/routers/jev-router).

## October 8, 2026 compatibility refresh

The refresh adds 87 endpoint-scoped chat entries, eight image entries, and three
speech entries to the existing catalogs. OpenRouter's public catalog supplies
51 of the chat additions; batch-only and image-output models are not added as
ordinary chat candidates. Existing recommended models, saved configurations,
and still-supported Nexa effort defaults are retained. Absence from a discovery
response is not retirement evidence.

- [Claude Haiku 5.5](https://platform.claude.com/docs/en/models/haiku-5-5/overview)
  uses adaptive thinking without manual budgets. Its default effort is medium;
  disabled thinking accepts low/medium/high, while xhigh/max require thinking.
  Disabled requests omit adaptive binding controls and reconcile changed
  thinking prefixes locally. The provider's account-binding rules still apply.
- OpenRouter reasoning uses the same normalized controls for Chat Completions
  and native-search Responses. Explicit OFF removes stale effort/budget fields;
  mandatory models remain enabled. The provider's default enabled state is
  distinct from the effort selected when a user explicitly enables thinking.
  Opaque replay is checked against its route and model at serialization.
- Token Plan's newly listed Qwen, DeepSeek, and GLM IDs keep their own regional
  controls. GLM uses top-level `clear_thinking:false` on this route, distinct
  from direct BigModel's nested field. Direct GLM FlashX is not promoted into
  Coding Plan. New SiliconFlow IDs do not inherit undocumented budget knobs.
- Ark Seed 2.1 preserves standalone `encrypted_content` frames as opaque
  replay, separately from visible reasoning summaries. Model/endpoint changes
  cannot forward the old state. See the [Ark Chat API](https://docs.volcengine.com/docs/ark/chat-api?lang=en).
- [Qwen Image 2.1 Pro](https://help.aliyun.com/zh/model-studio/qwen-image-generation-and-editing-api-reference)
  uses PNG and validated pixel/ratio limits on the existing single-image path;
  unsupported negative prompts are rejected. Nano Banana 2.1 uses Gemini
  GenerateContent's string-valued `imageConfig` fields.
- [Qwen Audio 3.1 TTS Flash](https://help.aliyun.com/zh/model-studio/qwen-audio-3-1-tts-flash)
  retains Beijing-only access and model-specific voices.
  [ElevenLabs dialogue](https://elevenlabs.io/docs/eleven-api/guides/how-to/websockets/realtime-tdd)
  adds v4 Turbo/v3 Conversational and requires the connection's final event
  before returning audio. Speech length checks use documented model limits;
  the previous generic 20,000-character rejection no longer hides larger limits.
- Runway Seedance 2.5 corrects 480p ratios and accepts 1080p and seeds under its
  [published schema](https://docs.dev.runwayml.com/openapi.json). The direct
  ByteDance API is documented but remains unselectable until Nexa has a
  production submission/recovery path. Other new video operations are not
  advertised as executable merely because their names appear in a directory.

The refresh uses official documentation and public catalog snapshots, request
fixtures, local HTTP/WebSocket servers, and browser settings tests. These checks
do not establish paid inference quality or access for a particular account.

## Model retirement

### September 30, 2026 catalog review

The review covered all 71 existing presets across chat, images, video, embeddings,
and speech, plus the subscription and ACP runtimes. Public sources verify model
contracts; account availability still comes from each authenticated runtime.
Existing provider defaults and saved selections stay in place; adding a gated
or preview model does not select it automatically.

- Added [GPT-6.1 Sol](https://developers.openai.com/api/docs/models/gpt-6.1-sol)
  on the direct Responses route, and nine new models verified through
  [OpenRouter's model API](https://openrouter.ai/api/v1/models), including Sol/Pro,
  GLM and Qwen Prime, Ember 1, Perceptron 1.5, Aion 3.5, and Solar Mini4. The
  individual model API's reasoning metadata determines each gateway's controls.
  Solar's default selection is `none`, matching its default-disabled reasoning.
- Added gated [MiniMax M3.1 Flash Preview](https://platform.minimax.io/docs/guides/text-generation)
  and [Claude Mythos 5.1](https://platform.claude.com/docs/en/models/mythos-5-1/overview).
  MiniMax M3.1 preserves separate `reasoning_content` through tool calls;
  M3 retains its own optional adaptive-thinking contract.
- Added Alibaba-hosted [DeepSeek V4.1 Flash](https://help.aliyun.com/zh/model-studio/deepseek-v4-1-flash),
  [GLM Flash/FlashX](https://help.aliyun.com/zh/model-studio/glm-zhipu),
  and [Step 5 Preview](https://help.aliyun.com/zh/model-studio/stepfun), with
  endpoint-specific reasoning fields. Their entries expose the text/image
  attachment transport implemented by Nexa.
- Added [Doubao Seed 2.1 and Evolving](https://docs.volcengine.com/docs/ark/model-list?lang=zh),
  [Mistral-hosted GLM 5.3/5.2](https://docs.mistral.ai/models/zai-glm-5-3), and
  the local [Gemma 4](https://ollama.com/library/gemma4) suggestion. For pages
  publishing abbreviated limits, Nexa uses conservative decimal budgets:
  Doubao 1,024,000/256,000 and Mistral GLM 1,000,000/128,000. It does not copy
  native Z.ai reasoning controls into Mistral's route or download local weights.
- Updated [Qwen Image 3.0](https://help.aliyun.com/zh/model-studio/qwen-image-generation-and-editing-api-reference)
  in Beijing and Singapore for the existing synchronous, single-image text-to-image
  tool. Added [MiniMax H3 Max](https://platform.minimax.io/docs/api-reference/video-generation-v2)
  with its own 480P/768P, 5–15 second limits and
  [output/reference pricing](https://platform.minimax.io/docs/guides/pricing-paygo).
- Added Cohere Embed v5 Pro/Fast and Qwen3.7 Embedding Flash; see
  [embedding providers](EMBEDDING_PROVIDERS.md). Speech and Live models continue
  to use their dedicated protocols, rather than appearing as chat completions.

The four Yi API models are retired following the
[platform shutdown](https://platform.lingyiwanwu.com/). Three Doubao IDs are
also retired: `doubao-seed-code-preview-251028`, `doubao-seed-1-6-251015`, and
`doubao-seed-1-6-flash-250828`. Seed 2.0 Pro remains selectable until its future
shutdown, and Gemini 2.5 is retained for eligible existing accounts. Sources:
[Ark retirement notice](https://docs.volcengine.com/docs/ark/model-deprecation-notice?lang=zh),
[Gemini lifecycle](https://ai.google.dev/gemini-api/docs/deprecations).

### Retirement behavior

Confirmed retirements are endpoint-scoped tombstones in the shared catalog.
They override old discovery caches and are excluded from desktop and phone
choices. The API provider boundary rejects both streaming and non-streaming
requests to those IDs, including documented retired aliases. Saved connections
remain editable with their original model ID and a suggested replacement;
Nexa does not select a differently priced model without a user selection.

`deprecated` and `legacy` do not mean unavailable. An announced future shutdown
does not remove a model early, and a missing account discovery result alone
does not establish retirement. A redirected retired ID also needs an explicit
replacement choice: its old model no longer serves requests even if the host
still accepts that spelling. Local and private deployments retain their own
availability authority.

For example, the [Moonshot model list](https://platform.kimi.ai/docs/models)
retires K2.5 and Moonshot V1, while [Alibaba's retirement notice](https://help.aliyun.com/zh/marketplace/three-party-direct-supply-model-kimi-k2-5-offline-notification)
applies to its third-party direct-supply route. Alibaba's separately hosted
`kimi-k2.5` is not the same route as `kimi/kimi-k2.5`. Similarly,
[DeepSeek's current service notice](https://api-docs.deepseek.com/quick_start/pricing/)
keeps V4 Pro available after the previously announced deadline. Record current
source links and verification dates when changing lifecycle facts.

## Select the right execution surface

| Task | Boundary |
| --- | --- |
| Normal API chat | Nexa owns the agent loop and dispatches available tools under current policies |
| Subscription chat | The official runtime owns its model loop; Nexa owns tool dispatch and durable conversation state |
| Subagent work | Configured API workers selected through `list_subagent_models` and `agent_config_id`; subscription child runtimes are not implemented |
| Dictation | A speech connection that actually supports the selected input and interim/final delivery mode |
| Live | A supported native realtime protocol or a configured vision plus streaming-transcription route; see [Voice and Live](LIVE.md) |
| Embeddings | A configured local or API embedding runtime; API embeddings disclose the text sent for embedding |
| Phone chat | Desktop-backed model choices and execution; the phone does not receive model credentials |

Controls changed during an active response apply according to their owning
next-turn selection contract; they must not mutate an already accepted provider
route mid-sample. A failed or unavailable route should report its actual failure.

## Troubleshooting

| Symptom | Next check |
| --- | --- |
| Signed in, but no subscription provider in the picker | Complete Add Provider and save the provider after enrollment |
| Models cannot be refreshed | Check the endpoint, account entitlement, and discovery support; retain or enter a documented model ID instead of inventing one |
| Reasoning or a media mode is unavailable | Inspect capabilities for the exact selected connection; model-family support is insufficient |
| Dictation only appears after recording stops | Check the adapter's delivery mode; a batch engine is not an interim streaming adapter |
| Context or output limits differ between connections | Check endpoint/model capacity authority and explicit overrides before changing orchestration budgets |

## Implementation and checks

- [Model choices](../apps/desktop/src/features/models/modelChoices.ts) and
  [desktop model resolution](../apps/desktop/src-tauri/src/commands/model_choices.rs).
- [External agent execution](../apps/desktop/src-tauri/src/agent_runtime): official
  subscription adapters and installed ACP agents; kept separate from API LLM adapters.
- [Catalog audit](../scripts/model-catalog-audit.mjs): run root
  `npm run catalog:audit` and `npm run catalog:audit:test` after catalog changes.
- [Provider settings browser coverage](../apps/desktop/e2e/settings-provider-models.spec.ts).

Static catalogs and mocked UI tests do not establish real account entitlement
or live provider availability. Keep those acceptance results separate.
