# Models and providers

[Documentation](../README.md) / [Category overview](README.md)

The catalog keeps provider-specific lifecycle and capability evidence. The latest
update adds Vertex AI and Bedrock connections, Jev 1.14 and the reviewed
OpenRouter Decisions models, Meta Muse Image generation and raster Recraft
Styles models. OpenAI's announced future retirements remain selectable with
`deprecated` metadata; retired entries stay out of active choices. Public
OpenRouter audits cover text, embeddings, images and decisions separately.
Anthropic discovery and health checks read authenticated `/v1/models` pages;
failure uses the curated fallback without claiming live discovery succeeded.

The current catalog also includes Mistral-hosted GLM 5.3 Flash, the paid
OpenRouter MiniMax M3.1 Flash route, and Qwen Audio 3.1 TTS Plus on the existing
speech transport. Flash TTS accepts the documented Beijing and Singapore
endpoints; Plus keeps its own voice list. The OpenRouter MiniMax free route
currently has no serving endpoints and stays as an unavailable historical ID,
with no automatic replacement by a paid model. Public catalogs and documentation
establish route metadata, not account entitlement or successful inference.

## Usage statistics

Settings → AI Usage shows a daily contribution heatmap for the past 365 days.
Provider, model and operation filters apply to it; the date range controls the
summary metrics and breakdown below. The separate horizontal trend is removed.
Heatmap intensity uses the canonical total, so reported reasoning details are not
counted again on top of an inclusive provider total.

The breakdown can group by provider or compare a model across providers. Public
vendor prefixes and Anthropic version punctuation are normalized for comparison,
while dated versions, private namespaces and route suffixes remain distinct.
Requests, tokens, exports and selected-range deletion use the same exact set of
underlying model IDs. Expand a row for cache, success, cost and per-request detail;
search narrows the displayed breakdown without changing the accounting filters.
Filter choices remain available after a selection returns no records.

## Connection and model identity

Gemini Argon uses native thinking levels and preserves the exact request across
`CONTINUATION` hops through the top-level `continuationToken`. A hop can contain
only thinking. Usage snapshots are cumulative inside a hop and additive across
hops; context occupancy is kept separate from repeated billed input. The adapter
does not replay checkpoint tokens after an uncertain transport failure or raise
the caller's output budget on `MAX_TOKENS`. Synchronous GenerateContent requests
are capped at the documented 32,768-token initial-request limit; streaming keeps
the supported 65,536-token limit. Sampling controls unsupported by Argon are
omitted, and cancellation drops the current hop and prevents the next one.

Nexa separates a saved connection, a selected model, and the capabilities of the
exact endpoint behind that connection. Model names alone do not authorize a
protocol, context limit, reasoning control, or credential route.

## Choose a connection

**GitHub Copilot API** is available in API models, separately from the SDK and ACP
external agents. It accepts a Copilot-enabled GitHub account token or an explicit
import from `gh auth login`. Refresh models after importing: available IDs, tools,
vision, reasoning levels and context limits come from the current account.
The saved token uses the normal encrypted provider credential store. Requests use
official Copilot HTTPS endpoints; arbitrary gateways are not accepted for this
credential type. Nexa owns execution, history and compaction for this direct entry.
See [External agents and subscriptions](SUBSCRIPTION_AGENTS.md) for ownership and
the distinction between readable summaries and opaque reasoning state.

The chat toolbar separates the provider icon, model, and reasoning controls.
Choose a provider to restore that connection's saved model and reasoning settings;
choose the model label to browse only its models. Subscription and ACP catalogs
still determine which saved choices are currently available.

### October 24, 2026 additions

- Direct Moonshot `kimi-k3.1-pro` and `kimi-k3.1-pro-highspeed` use
  `max_completion_tokens`, always-on reasoning with `low`/`high`/`max`, and the
  provider's `max` default. Fixed sampling fields are omitted. OpenRouter's
  `moonshotai/kimi-k3.1-pro` retains its normalized router contract. Existing K3
  saved/default effort remains unchanged. K2.6/K2.7 entries are deprecated,
  not removed before their announced December retirement.
  [Moonshot models](https://platform.kimi.ai/docs/models),
  [K3.1 guide](https://platform.kimi.ai/docs/guide/kimi-k3-1-pro-quickstart).
- TypeSafe's latest alias now resolves to stable Jev 1.14. OpenRouter adds its
  public latest alias and three decision-only models; they do not become chat
  models. NVIDIA Nemotron 3.5 embedding entries keep distinct 2,048/4,096-vector
  identities without inventing a dimension-override request option.
  [TypeSafe models](https://docs.typesafe.ai/models),
  [OpenRouter catalog](https://openrouter.ai/api/v1/models).
- Mistral's public-preview Voxtral 3 file-transcription models and TTS use their
  native request fields. Transcription omits the unsupported OpenAI response
  format field on the official endpoint. TTS uses `voice_id` and decodes the
  bounded `audio_data` response. These presets do not imply support for Mistral's
  different realtime protocol, vector-image output or another provider's video
  transport. [Mistral changelog](https://docs.mistral.ai/resources/changelogs).

The audit covers all static endpoint projections and anonymous public catalogs
where available. Account-specific entitlement and paid live inference are
separate checks; an inaccessible lifecycle page is not evidence of retirement.

### October 22, 2026 catalog audit

The shared catalog validates 85 endpoints and 1,126 model projections. The public
OpenRouter text catalog contains every active curated route; already-retired routes
remain tombstones rather than reappearing through saved configurations. Upcoming
retirements remain deprecated until their published shutdown dates.

New reviewed routes include [Recursive Flint on OpenRouter](https://openrouter.ai/recursive/flint-v1.0:free),
[GLM-5.3-FlashX-Legal](https://docs.bigmodel.cn/cn/guide/models/vlm/glm-5.3-flash)
(application required), [SiliconFlow DeepSeek V4.1 Flash 1017](https://docs.siliconflow.cn/docs/release-notes/overview),
and Alibaba's [pinned DeepSeek V4.1 Flash 1017](https://help.aliyun.com/zh/model-studio/deepseek-v4-1-flash)
on its Beijing and Singapore connections,
and [MiniMax M3.1 Flash direct supply](https://help.aliyun.com/zh/model-studio/minimax-api-by-minimax).
The [GLM-Legal application](https://z.ai/consultation/legal) remains required.
Alibaba's [MiMo V2.5 Pro route](https://help.aliyun.com/zh/model-studio/mimo),
retired on October 21, is hidden while retaining its replacement metadata.
Alibaba MiniMax keeps thinking on, exposes its five documented effort levels,
and replays `reasoning_content` alongside tool calls. Its fixed sampling and
unsupported stop/parallel-tool controls are omitted from requests. SiliconFlow's new route
retains provider defaults without inheriting the generic numerical thinking budget.

Alibaba's Beijing-only [Ling 3.1 Flash](https://help.aliyun.com/zh/model-studio/antdigital)
requires workspace activation. Its optional thinking defaults on; Nexa preserves
reasoning history and omits unsupported effort, numeric budget and parallel-tool
fields. Singapore and Token Plan do not inherit this route's contract.
The image fallback catalog also includes
[Riverflow V2.6 Pro](https://openrouter.ai/sourceful/riverflow-v2.6-pro), including
reference images and its per-model raster formats and sizes. Google's direct
`gemini-3.1-flash-image` remains selectable but is marked deprecated in favor of
`gemini-nano-banana-2.1`, following the
[official lifecycle notice](https://ai.google.dev/gemini-api/docs/changelog).

This audit checks public documentation, shared projections and public discovery.
It does not establish paid inference, account entitlement, local model installation,
or authenticated discovery for credentials unavailable to the audit process.

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

### Vertex AI and Amazon Bedrock

Both providers appear under **Settings → AI Providers** with their own icons.
Their credentials are saved in the existing endpoint-scoped secret store.

| Connection | Base URL and credential |
| --- | --- |
| Vertex Gemini Express | `https://aiplatform.googleapis.com/v1/publishers/google`; Google Cloud API key |
| Vertex Gemini project | `https://LOCATION-aiplatform.googleapis.com/v1/projects/PROJECT/locations/LOCATION/publishers/google`; OAuth access token with Cloud Platform scope |
| Vertex Claude | The same project/location form ending in `/publishers/anthropic`; OAuth access token and enabled partner-model access |
| Bedrock Converse | `https://bedrock-runtime.REGION.amazonaws.com`; Amazon Bedrock API key (Bearer token) |

For Vertex's `global` location, use the unprefixed `aiplatform.googleapis.com`
host. Replace project placeholders before sending a request. Manually supplied
OAuth tokens expire: this connection does not refresh ADC, service-account,
SSO or IAM credentials. Bedrock AWS access-key IDs are not bearer API keys.

Vertex Gemini uses GenerateContent/SSE; Vertex Claude uses rawPredict and
streamRawPredict with the native Claude body. Bedrock uses Converse and the
AWS binary event stream, including late errors and trailing usage metadata.
Native signed reasoning and tool-use IDs remain paired with their exact route.
The same provider-neutral tool loop consumes these transports.

Google Search uses the existing **Settings → Web Search** execution mode.
Verified Gemini 3/4 Developer API models can combine native search and workspace
tools; Nexa enables tool context circulation and retains the signed server-tool
parts for replay. Vertex Gemini and Gemini 2.5 support native search in a
search-only request. With workspace tools, **Auto/Hybrid** retains Nexa Router
search, while **Provider Native** explains the unsupported combination instead
of dropping tools or sending an invalid request. Capability checks follow the
actual endpoint and model, including Vertex project/location routes. See
[Gemini tool combinations](https://ai.google.dev/gemini-api/docs/generate-content/tool-combination)
and [Vertex grounding limits](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/grounding/grounding-with-google-search).

Bedrock model selection accepts regional or global inference-profile IDs and
full profile ARNs without rewriting them. Curated global profiles are examples;
account, source-region and model permissions still determine availability.
Discovery permission and inference permission are separate. The configured
connection test is the explicit model probe; a catalog listing alone is not
proof of successful inference. Native PDF input is not advertised for these
cloud routes; existing local extraction remains available.

Lifecycle metadata also stays endpoint scoped. For example, Vertex's Gemini
2.5 retirement schedule does not retire the same IDs on the Gemini Developer
API. Consult the provider's current lifecycle page before migrating a saved
connection: [Vertex model lifecycle](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/model-versions),
[Bedrock lifecycle](https://docs.aws.amazon.com/bedrock/latest/userguide/model-lifecycle.html),
[Gemini Developer API deprecations](https://ai.google.dev/gemini-api/docs/deprecations).

Implementation and focused wire tests live in
[Vertex routing](../../crates/core/src/llm/vertex.rs),
[Bedrock transport](../../crates/core/src/llm/bedrock.rs), and
[Bedrock tests](../../crates/core/src/llm/bedrock/tests.rs).

## OpenRouter credentials

Use a regular inference API key from [OpenRouter API keys](https://openrouter.ai/settings/keys).
Management/provisioning keys can query key metadata but cannot call models. This
can produce HTTP 401 `User not found` in both Check connection and Chat even when
the key-information endpoint accepts the same credential. Nexa preserves the 401
and explains the key-type distinction; it does not retry inference or create or
replace account credentials automatically. Invalid or expired inference keys and
service-side authentication failures can also return 401, so the message alone
does not prove which case applies.

Source: [OpenRouter management keys](https://openrouter.ai/docs/guides/overview/auth/management-api-keys).

## Capability resolution

The shared [provider presets](../../shared/provider-presets.json),
[core provider catalog](../../crates/core/src/provider_catalog.rs), and
[capability registry](../../crates/core/src/capability_registry) define known
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
worker budgets, see [Orchestration runtime](../runtime/ORCHESTRATION_RUNTIME.md).

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

The [separate System One catalog](../../shared/system-one-provider-presets.json)
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
- Ark GLM 5.3 Flash and DeepSeek V4.1 Flash have separate native thinking
  controls and textual reasoning replay; they never inherit Seed ciphertext or
  direct-vendor fields. Jev Router and Token Plan Auto preserve returned
  reasoning without inventing router-level thinking controls.
- [Mistral Large 4](https://docs.mistral.ai/studio/conversations/reasoning)
  supports explicit none/high effort and Mistral thinking content chunks.
  Nexa preserves omission when no override is supplied; the upstream default
  for an omitted effort is not assumed.
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
| Subagent work | API, Copilot and Codex workers inherit the active account and model; `list_subagent_models` and an explicit `agent_config_id` select another route |
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

The October 18 refresh updates the canonical DeepSeek Flash label to
DeepSeek-V4.1-Flash-1017 and adds the new compatible OpenRouter text and image
routes. Krea K3 Edit uses the existing OpenRouter Images endpoint with 1–8
local references. `generate_image.reference_image_paths` follows the same file
access and pixel-disclosure approval as native image reads, preserves accepted
PNG/JPEG/WebP bytes, and sends typed `input_references` content parts. Nexa caps
inputs at 12 MiB each, 32 MiB total, 8192 pixels per edge and 16 million pixels
per image; these are host resource limits, not advertised provider limits.
Missing references and unsupported options fail before generation. See the
[OpenRouter Images OpenAPI](https://openrouter.ai/docs/openapi/openapi.yaml).
DeepSeek Search and Fetch are configured in the existing web-search
settings, using their own saved bearer key and the existing provider priority.
See [DeepSeek's model contract](https://api-docs.deepseek.com/quick_start/pricing/)
and [Search/Fetch guide](https://api-docs.deepseek.com/guides/search/).
SiliconFlow's retired `diffusiongemma` decision route is no longer selectable;
saved selections produce a migration error instead of silently choosing a
different model. Deprecation notices with a future shutdown date do not remove
currently supported routes.

Copilot reasoning controls follow the official runtime's model capabilities and
advertised effort IDs. An explicit `supports.reasoningEffort: false` takes
precedence over a stale effort list. A model without configurable levels shows
provider-managed reasoning. Native Google capabilities are not copied into a
Copilot model with the same family name; see the
[Copilot model reference](https://docs.github.com/en/copilot/reference/ai-models/supported-models).

The October 15, 2026 catalog audit covers 81 API/local presets and 1,068 model
projections. Public discovery probes cover OpenRouter text, images and its
separate embedding endpoint; the report lists unprobed endpoints explicitly.
Public catalog presence, implemented request contracts and account entitlement
are separate evidence levels. Missing listings never remove a model by themselves.

This update adds Ollama Qwen 3.8, pinned Alibaba decision-model choices, four
current OpenRouter embedding routes, native Gemini TTS/ASR and Meta file ASR.
The existing Runway task runtime now serves 25 models with 76 explicit operation
contracts, including 22 additional models. Per-operation ratios, duration,
reference media, audio, seed and video field names come from the
[Runway OpenAPI](https://docs.dev.runwayml.com/openapi.json). Unsupported optional
features such as automatic duration, multishot and timed Aleph keyframes are not
advertised. Unknown prices remain unknown instead of inheriting another model's
tariff. Local fixture tests do not establish paid-generation quality or access.

OpenRouter Gemini Embedding 2 uses distinct query/document text preparation.
Changing those semantics changes its vector-space identity and requires reindexing;
existing vectors are never silently reinterpreted. NVIDIA Nemotron VL retains the
router's advertised capacity separately from its conservative serving capacity,
which controls document splitting. Other missing router embedding candidates
include older compatibility models: a dot-product model is not advertised through
Nexa's cosine retrieval, and an undocumented Gemini 001 gateway task mapping is
not guessed. Existing selections and defaults are preserved.

OpenAI `tts-1-hd` retains availability until its documented January 6, 2027
shutdown. The audit found no additional confirmed immediate retirements beyond
the existing endpoint-scoped tombstones.

- [Model choices](../../apps/desktop/src/features/models/modelChoices.ts) and
  [desktop model resolution](../../apps/desktop/src-tauri/src/commands/model_choices.rs).
- [External agent execution](../../apps/desktop/src-tauri/src/agent_runtime): official
  subscription adapters and installed ACP agents; kept separate from API LLM adapters.
- [Catalog audit](../../scripts/model-catalog-audit.mjs): run root
  `npm run catalog:audit` and `npm run catalog:audit:test` after catalog changes.
- [Provider settings browser coverage](../../apps/desktop/e2e/settings-provider-models.spec.ts).

Static catalogs and mocked UI tests do not establish real account entitlement
or live provider availability. Keep those acceptance results separate.
