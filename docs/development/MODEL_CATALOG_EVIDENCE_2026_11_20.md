# Model catalog evidence: November 20, 2026

[Development documentation](README.md) · [Maintenance policy](MODEL_CATALOG_MAINTENANCE.md)

Baseline: `dda9808790c402186a2b539311f830b0d9bc366a`. No open PR overlapped
at the start of this review. Public catalogs establish advertised routes, not
account entitlement or successful inference. No inference, agent installation,
account modification, default migration, merge or release was performed.

## Changes

- OpenRouter `z-ai/glm-5.1` explicitly withdrew its November 20 expiration:
  `expiration_date: null`; its endpoint response listed ten backing providers
  with status 0. Clear only that route's stale date/deprecated marker. Direct
  Z.ai, BigModel and Ark lifecycles are independent. [Catalog](https://openrouter.ai/api/v1/models),
  [backing providers](https://openrouter.ai/api/v1/models/z-ai/glm-5.1/endpoints).
- Add OpenRouter `mistralai/pixtral-large-2502`: text/image input, text output,
  128000 context and 102400 maximum output, currently backed by Amazon Bedrock.
  Its complete parameter list does not advertise tools, structured output or
  reasoning; do not inherit those from the original Mistral route.
  [Endpoint contract](https://openrouter.ai/api/v1/models/mistralai/pixtral-large-2502/endpoints).
- Add Beijing-only `kimi-k3.1-pro-prime` as an explicit, non-recommended,
  account-enabled selection. Official Prime documentation states the base
  model's capabilities/limits apply. Do not change the default or expand this
  paid speed route into Singapore or Token Plan. Workspace hosts retain their
  existing endpoint/credential identity.
  [Release list](https://help.aliyun.com/zh/model-studio/newly-released-models),
  [Prime contract](https://help.aliyun.com/zh/model-studio/prime-mode),
  [base model](https://help.aliyun.com/zh/model-studio/kimi-k3-1-pro).
- Lower OpenRouter's exact `~deepseek/deepseek-v4-flash-latest` output fallback
  from 943718 to the current 384000. Refresh the explicitly advertised parameter
  sets of `tencent/hy4` and `openai/gpt-oss-safeguard-20b`. No price is inferred.
- Fix the audit comparator to report an explicit null expiration withdrawing a
  saved date. Missing, undefined, empty and malformed metadata remain unknown;
  null alone never automatically changes a lifecycle. Regression fixtures cover
  both positive and negative cases.

## Remaining findings and boundaries

The public audit still reports eight higher top-provider output limits. Keep
existing conservative limits: MiMo V2.5 Pro and DeepSeek V4 Flash 0731 already
varied by provider/read; Qwen3.5 9B/27B, GPT OSS120B and three router products also
have newly higher advertised maxima. These do not justify silently raising a
user's requested output budget. Three missing chat IDs remain review candidates,
not proof of retirement: `cohere/north-mini-code:free`,
`thinkingmachines/inkling-2-small:free`, `x-ai/grok-build-0.1`.

The corrected null-expiration check also found OpenRouter `z-ai/glm-4.7` and
Decisions `liquid/d1-ultra`. Independent reads corroborated explicit null dates;
GLM 4.7 has six backing endpoints (two are DeepInfra), D1 Ultra has Liquid.
Clear those scoped announcements too; preserve D1.1's separate deadline.
[GLM endpoints](https://openrouter.ai/api/v1/models/z-ai/glm-4.7/endpoints),
[D1 Ultra endpoints](https://openrouter.ai/api/v1/models/liquid/d1-ultra/endpoints).

Vertex `gemini-3.6-flash` remains deprecated with its announced November 19
retirement date. The official lifecycle page still has not moved it to the
Retired table; its model card still uses scheduled wording. No authenticated
project/region call was made. Google Developer API and OpenRouter remain
independent routes. [Cloud lifecycle](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/model-versions),
[model card](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/gemini/3-6-flash),
[API comparison](https://firebase.google.com/docs/ai-logic/models).

Ark's four pending November 24 chat retirements remains pending until
14:00 UTC+8 on that date. EOM-only embeddings do not imply existing service
shutdown or permission to rebuild indices. Mistral's model cards distinguish
deprecation announcement from retirement; `deprecationDate` currently mixes
these meanings across providers. Do not turn an announcement date into a
removal or infer exact shutdown dates by adding six months. Retiree replacement
links pointing to a still-deprecated model need explicit review, not automatic
paid-route migration. [Ark notice](https://docs.volcengine.com/docs/ark/model-deprecation-notice?lang=zh),
[Mistral lifecycle](https://docs.mistral.ai/inference/model-lifecycle).

OpenAI announced a January 11, 2027 change limited to GPT-5.2/5.4/5.4-mini
with EU data residency and fast/priority processing, not standard processing.
Do not globally deprecate those model IDs. Azure retirement is independently
scoped to deployment version, region and SKU; no Azure deployment inventory was
queried. [OpenAI notice](https://developers.openai.com/api/docs/deprecations),
[Azure schedule](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/model-retirement-schedule).

New OpenAI `gpt-live-1.1` uses `/v1/live/sessions`; Google Genie 4 uses
Interactions v1beta real-time world generation. Neither is a drop-in chat,
transcription, TTS or existing asynchronous video model.
[OpenAI changes](https://developers.openai.com/api/docs/changelog),
[Google changes](https://aistudio.google.com/docs/changelog).

## Coverage

The machine audit enumerated 94 operation endpoints and 1512 model projections
after changes. Six public probes succeeded, zero failed: OpenRouter chat 478,
image 70, embedding 38, decision 43, video 35; Jina 30. The remaining 88 endpoints
are explicitly unverified by live discovery: 22 need credentials, 60 have no
operation-specific discovery contract in this auditor, six use unsupported
or runtime discovery. Success of this command is not full provider verification.

Public official releases/lifecycle references were revisited for OpenAI,
Anthropic, Google/Vertex, Bedrock, Azure, Mistral, DeepSeek, xAI, MiniMax,
Alibaba Beijing/international, SiliconFlow China/international and BigModel/Z.ai.
These documentary checks do not establish account access. Subscription catalogs
remain signed-in runtime data; local models are installed-runtime data. No
upstream model release was treated as locally installed.

The following table is generated from this run's actual endpoint report; a
skipped row does not imply the provider is unavailable.

| Endpoint / region scope | Live discovery result |
| --- | --- |
| `text:github-copilot-api` | discovery_not_supported |
| `text:openai` | credential_not_configured |
| `text:openrouter` | public inventory: 478 |
| `text:anthropic` | credential_not_configured |
| `text:vertex` | discovery_not_supported |
| `text:vertex-anthropic` | discovery_not_supported |
| `text:bedrock` | discovery_not_supported |
| `text:google` | credential_not_configured |
| `text:deepseek` | credential_not_configured |
| `text:xai` | credential_not_configured |
| `text:xiaomi-mimo` | credential_not_configured |
| `text:meta-model-api` | credential_not_configured |
| `text:minimax` | credential_not_configured |
| `text:mistral` | credential_not_configured |
| `text:ollama` | discovery_not_supported |
| `text:lmstudio` | discovery_not_supported |
| `text:zhipu` | credential_not_configured |
| `text:zhipu-intl` | credential_not_configured |
| `text:moonshot` | credential_not_configured |
| `text:siliconflow` | credential_not_configured |
| `text:qwen-token-plan-cn` | credential_not_configured |
| `text:qwen-token-plan-global` | credential_not_configured |
| `text:qwen-cloud-intl` | credential_not_configured |
| `text:alibaba-model-studio` | credential_not_configured |
| `text:doubao` | credential_not_configured |
| `text:yi` | credential_not_configured |
| `text:baichuan` | credential_not_configured |
| `image:openai` | operation_catalog_not_supported |
| `image:xai` | operation_catalog_not_supported |
| `image:google-gemini` | operation_catalog_not_supported |
| `image:qwen-dashscope-cn` | operation_catalog_not_supported |
| `image:qwen-dashscope-intl` | operation_catalog_not_supported |
| `image:zhipu-cogview` | operation_catalog_not_supported |
| `image:openrouter-images` | public inventory: 70 |
| `image:custom-openai-images` | operation_catalog_not_supported |
| `image:meta-model-api` | operation_catalog_not_supported |
| `embedding:openai` | operation_catalog_not_supported |
| `embedding:alibaba-model-studio-cn` | operation_catalog_not_supported |
| `embedding:alibaba-model-studio-sg` | operation_catalog_not_supported |
| `embedding:alibaba-model-studio-hk` | operation_catalog_not_supported |
| `embedding:jina` | public inventory: 30 |
| `embedding:mistral` | operation_catalog_not_supported |
| `embedding:siliconflow-cn` | operation_catalog_not_supported |
| `embedding:siliconflow-international` | operation_catalog_not_supported |
| `embedding:zhipu` | operation_catalog_not_supported |
| `embedding:baidu-qianfan` | credential_not_configured |
| `embedding:voyage` | operation_catalog_not_supported |
| `embedding:cohere` | operation_catalog_not_supported |
| `embedding:google-gemini` | credential_not_configured |
| `embedding:openrouter` | public inventory: 38 |
| `embedding:ollama` | operation_catalog_not_supported |
| `embedding:lmstudio` | operation_catalog_not_supported |
| `embedding:custom` | operation_catalog_not_supported |
| `speech_to_text:local-whisper` | operation_catalog_not_supported |
| `speech_to_text:openai` | operation_catalog_not_supported |
| `speech_to_text:openai-live` | operation_catalog_not_supported |
| `speech_to_text:groq` | operation_catalog_not_supported |
| `speech_to_text:alibaba-qwen-audio-streaming` | operation_catalog_not_supported |
| `speech_to_text:alibaba-qwen-audio` | operation_catalog_not_supported |
| `speech_to_text:alibaba-qwen-asr` | operation_catalog_not_supported |
| `speech_to_text:alibaba-qwen-realtime` | operation_catalog_not_supported |
| `speech_to_text:siliconflow` | operation_catalog_not_supported |
| `speech_to_text:sherpa-sense-voice` | operation_catalog_not_supported |
| `speech_to_text:sherpa-zipformer` | operation_catalog_not_supported |
| `speech_to_text:custom-openai` | operation_catalog_not_supported |
| `speech_to_text:google-transcribe` | operation_catalog_not_supported |
| `speech_to_text:meta-transcribe` | operation_catalog_not_supported |
| `speech_to_text:google-transcribe-live` | operation_catalog_not_supported |
| `speech_to_text:mistral-realtime` | operation_catalog_not_supported |
| `speech_to_text:mistral` | operation_catalog_not_supported |
| `text_to_speech:openai` | operation_catalog_not_supported |
| `text_to_speech:groq` | operation_catalog_not_supported |
| `text_to_speech:elevenlabs` | operation_catalog_not_supported |
| `text_to_speech:azure-speech` | operation_catalog_not_supported |
| `text_to_speech:minimax` | operation_catalog_not_supported |
| `text_to_speech:qwen-audio-generation` | operation_catalog_not_supported |
| `text_to_speech:dashscope-cosyvoice` | operation_catalog_not_supported |
| `text_to_speech:sherpa-onnx` | operation_catalog_not_supported |
| `text_to_speech:siliconflow` | operation_catalog_not_supported |
| `text_to_speech:google-gemini` | operation_catalog_not_supported |
| `text_to_speech:mistral-realtime` | operation_catalog_not_supported |
| `text_to_speech:mistral` | operation_catalog_not_supported |
| `decision:typesafe` | operation_catalog_not_supported |
| `decision:openai` | operation_catalog_not_supported |
| `decision:openrouter` | public inventory: 43 |
| `decision:siliconflow-cn` | operation_catalog_not_supported |
| `decision:siliconflow-international` | operation_catalog_not_supported |
| `decision:alibaba-model-studio-cn` | operation_catalog_not_supported |
| `decision:alibaba-model-studio-intl` | operation_catalog_not_supported |
| `video:minimax-global` | operation_catalog_not_supported |
| `video:minimax-hailuo-global` | operation_catalog_not_supported |
| `video:runway-global` | operation_catalog_not_supported |
| `video:bytedance-direct-watchlist` | operation_catalog_not_supported |
| `video:openrouter-video` | public inventory: 35 |

## Validation

Catalog audit fixtures and four registry/Runway contracts passed. The November
18/20 provider regression suite passed all eleven cases. The new regression file
is included in the existing frontend contract command. No Rust toolchain is
available locally; native/full frontend validation is left to exact-head CI,
not claimed from these focused tests.

Additional public checks found no Runway video ID differences against its live
OpenAPI or Jina embedding/CLIP gaps. Cohere, Voyage, Groq, ElevenLabs and Meta
public pages showed no confirmed new lifecycle change.
[Runway](https://docs.dev.runwayml.com/openapi.json), [Jina](https://api.jina.ai/v1/models).

ACP has 46 configurations (44 registry identities plus Hermes/custom). Its
public registry showed twelve version candidates, but direct HTTP returned 403
and complete package/archive digest verification was not achieved. No pins or
hashes were blindly changed; no agents were installed. Candidates: Claude
0.109.0, Codex 2.8.0, Cursor 2026.11.19, Factory Droid 1.15.0, Gemini 0.70.0,
Grok Build 1.0.91, Junie 4100.10.0, Kimchi 1.37.2, Kimi Code 2.7.0,
MiniMax Code 0.7.8, Muse Code 1.4.24 and Qoder 1.1.93.
[Registry](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json).

Not every operation guide was re-read: Alibaba image/voice regional contracts,
SiliconFlow speech variants, Baidu/BigModel embeddings and Azure voices remain
verification gaps beyond the explicit checks above. This is not a claim of
exhaustive live provider coverage.
