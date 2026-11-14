# Model catalog audit and coverage

[Development guides](README.md) · [Documentation home](../README.md)

## Evidence boundaries

Catalog maintenance covers every endpoint preset in the seven shared catalogs, including aggregators and regional hosted routes. Model identity is the endpoint plus the exact wire ID; a native model release does not establish availability or reasoning controls on a reseller. Inventory, documented support, account discovery, and successful inference are separate evidence levels. Unknown fields remain unknown.

The 2026-11-14 review inventories **92 endpoint presets and 1,412 model projections**, including retained retirement tombstones. Six public endpoint catalogs were fetched successfully; protected catalogs were not authenticated and no inference was purchased. A successful public catalog fetch verifies advertised inventory and metadata, not account entitlement, every input modality in Nexa, or inference success.

| Surface | Endpoint presets | Model projections |
| --- | ---: | ---: |
| Text | 27 | 1021 |
| Image | 9 | 110 |
| Embedding | 15 | 95 |
| Speech to text | 17 | 28 |
| Text to speech | 12 | 34 |
| Decision | 7 | 57 |
| Video | 5 | 67 |

## Aggregators and managed hosting

| Endpoint or surface | First-party evidence checked | Result and remaining limit |
| --- | --- | --- |
| OpenRouter text | [Public models API](https://openrouter.ai/api/v1/models) | 471 advertised rows, including 87 batch-only rows. Every streaming chat ID already had a local entry; updated routed limits, modalities, tools, structured output, and explicit reasoning metadata. Gemini 4 Carbon now exposes mandatory low/medium/high reasoning; Hydrogen exposes low/medium/high/xhigh. Batch-only IDs stay outside the chat picker. |
| OpenRouter image | [Image models API](https://openrouter.ai/api/v1/images/models) | All 68 advertised IDs accounted for; refreshed supported parameters, output formats, input references, and modality metadata without duplicating the image runtime. |
| OpenRouter embeddings | [Embedding models API](https://openrouter.ai/api/v1/embeddings/models) | All 39 advertised IDs accounted for; corrected routed context limits. The application embedding input remains text. |
| OpenRouter Decisions | [Decisions models API](https://openrouter.ai/api/v1/models?output_modalities=decisions&endpoints=/api/alpha/decisions) | All 40 advertised IDs accounted for, matching resolved aliases separately; corrected five context limits and refreshed documented input modalities. |
| OpenRouter video | [Video models API](https://openrouter.ai/api/v1/videos/models) | All 32 advertised IDs accounted for. Video operations retain their separate supported operation contracts. |
| SiliconFlow China chat | [Official model inventory](https://www.siliconflow.cn/models), [release notices](https://docs.siliconflow.cn/docs/release-notes/overview) | Reviewed the public inventory payload: 91 model records, including 61 chat records. Added 44 omitted chat IDs with documented context, vision, tool and JSON support. Retained existing past retirements; November 16 retirements remain deprecated until that date. Unknown hosted reasoning controls do not inherit native profiles. Authenticated `/models` returned 401. |
| SiliconFlow embeddings | [Embedding API](https://docs.siliconflow.cn/docs/api/embeddings-post), [Qwen vector size](https://huggingface.co/Qwen/Qwen3-VL-Embedding-8B), [BGE M3 vector size](https://huggingface.co/BAAI/bge-m3), [BGE English vector size](https://huggingface.co/BAAI/bge-large-en-v1.5) | Added the four missing compatible text embedding models: Qwen3 VL 8B, BGE M3, Pro BGE M3, and BGE Large English. All eight public inventory embedding IDs are represented; unknown hosted dimension overrides remain disabled. |
| SiliconFlow speech and Decisions | [Transcription API](https://docs.siliconflow.cn/docs/api/audio-transcriptions-post), [release notices](https://docs.siliconflow.cn/docs/release-notes/overview) | Kept the documented transcription whitelist and both TTS models. Marketing inventory lists newer ASR models without a matching transcription API contract; these require endpoint confirmation before selection. The discontinued diffusiongemma Decisions route is absent. International models remain independently scoped. |
| Alibaba Beijing PAYG | [Model overview](https://www.alibabacloud.com/help/en/model-studio/models), [releases by deployment scope](https://www.alibabacloud.com/help/en/model-studio/newly-released-models) | Checked Qwen and hosted DeepSeek, GLM, Kimi, MiniMax, MiMo and Step families separately from their native endpoints. Current hosted entries stay scoped to the documented service and region. Protected `/models` returned 401. |
| Alibaba Singapore PAYG | [International releases](https://www.alibabacloud.com/help/en/model-studio/newly-released-models) | Added omitted exact hosted IDs deepseek-v4-pro-0813, ZHIPU/GLM-5.3, ZHIPU/GLM-5.2, and kimi/kimi-k3. No guessed output limits or reasoning controls were copied from Beijing/native routes. Protected `/models` returned 401. |
| Qwen Token Plan Beijing | [Beijing Qwen4 contract](https://help.aliyun.com/zh/model-studio/qwen4-max-preview) | Corrected Qwen4 effort choices to low/medium/xhigh. Preview ends November 25 at 24:00 UTC+8; PAYG access is excluded. |
| Qwen Token Plan Singapore | [Singapore Qwen4 contract](https://www.alibabacloud.com/help/en/model-studio/qwen4-max-preview) | Added qwen4-max-preview on the explicitly documented Singapore Token Plan endpoint, with independent regional identity. Protected `/models` returned 401. |
| Doubao / Volcengine Ark | [Current model list](https://docs.volcengine.com/docs/ark/model-list?lang=zh), [release notices](https://docs.volcengine.com/docs/ark/model-release-announcement?lang=zh), [retirement schedule](https://docs.volcengine.com/docs/ark/model-deprecation-notice?lang=zh) | Cross-checked current DeepSeek, GLM and Kimi hosted versions. Added three still-callable account-enabled snapshots scheduled for November 24 retirement, plus the documented translation model. Already retired October snapshots were not restored from stale model-list rows. Protected `/models` returned 401. |
| Mistral hosted models | [Models overview](https://docs.mistral.ai/models), [Large 4 GA](https://docs.mistral.ai/models/mistral-large-4-0) | Checked hosted GLM 5.3/Flash and DeepSeek V4.1 Flash independently. Updated Mistral Large 4 from preview to active GA. Protected `/models` returned 401. |
| Google Vertex Gemini and Anthropic | [Model lifecycle](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/model-versions) | Gemini 4 and Claude 5.5/Fable 5.2 entries retained with account/region gates. Native Google or Anthropic retirement dates do not automatically retire managed routes; project credentials are needed for a live account catalog. |
| Amazon Bedrock | [Managed model inventory](https://docs.aws.amazon.com/bedrock/latest/userguide/model-cards.html) | Existing model IDs and regional inference profiles remain distinct from upstream IDs. AWS credentials and regional entitlement are required to validate account availability. |
| Groq speech | [Supported models](https://console.groq.com/docs/models) | Checked the existing Whisper and Orpheus speech surfaces. There is no built-in Groq text preset in this repository. |

Together, Fireworks, Novita and ModelScope are not built-in provider endpoint presets in this snapshot. Models from those organizations may appear through OpenRouter; those IDs are audited under OpenRouter rather than advertised as separate native integrations. Custom endpoints can still discover their own account catalogs.

## Other provider checks

The native [Google model list](https://aistudio.google.com/docs/models) confirms the Carbon, Argon and Hydrogen family; synchronous output limits stay distinct from long decode and router limits. The [DeepSeek pricing and model table](https://api-docs.deepseek.com/quick_start/pricing/) confirms the Flash 1017 upgrade and compatibility aliases. [Claude lifecycle notices](https://platform.claude.com/docs/en/about-claude/model-deprecations) confirm existing tombstones and future Sonnet 4.5 retirement; supported legacy models are retained.

[Zhipu documentation](https://docs.bigmodel.cn/cn/guide/start/model-overview) supplies missing native limits. Added the omitted, tool-capable [GLM-4.7-Flash](https://docs.bigmodel.cn/cn/guide/models/free/glm-4.7-flash) and [GLM-4.6V-Flash](https://docs.bigmodel.cn/cn/guide/models/free/glm-4.6v-flash) chat entries. [Kimi](https://platform.kimi.ai/docs/models), [MiniMax](https://platform.minimax.io/docs/guides/models-intro), [xAI](https://docs.x.ai/developers/models), [MiMo](https://mimo.mi.com/models/en-US/mimo-v2.6-pro), [Meta](https://dev.meta.ai/docs/muse-ember), and [OpenAI](https://developers.openai.com/api/docs/models) first-party pages were reviewed without asserting account-level inference.

[Jina public inventory](https://api.jina.ai/v1/models) returned 30 records across embedding, reranking, OCR and reader families. Added the omitted legacy single-vector English embedding, retained canonical unprefixed embedding wire IDs, and added namespaced discovery aliases. [ColBERT multi-vector](https://huggingface.co/jinaai/jina-colbert-v2), OCR and reranker models are excluded from the single-vector embedding adapter. [Cohere](https://docs.cohere.com/docs/cohere-embed), [Voyage](https://docs.voyageai.com/docs/embeddings), [ElevenLabs](https://elevenlabs.io/docs/overview/models), [TypeSafe](https://docs.typesafe.ai/models), and [Runway](https://docs.dev.runwayml.com/guides/models/) documentation was also checked.

## Complete endpoint inventory

Counts include tombstones. **Public API** means an anonymous catalog response was compared. **Docs / credentials** means relevant first-party documentation was checked, while account discovery or inference remains unverified. **Credentials / retained** means configuration was inventoried but current model availability needs account access or further endpoint-specific evidence. **Local** and **Custom** depend on the user runtime/endpoint.

| Surface | Preset ID | Models | Verification boundary |
| --- | --- | ---: | --- |
| Text | `github-copilot-api` | 0 | Credentials / retained |
| Text | `openai` | 42 | Docs / credentials |
| Text | `openrouter` | 548 | Public API |
| Text | `anthropic` | 28 | Docs / credentials |
| Text | `vertex` | 15 | Docs / credentials |
| Text | `vertex-anthropic` | 4 | Docs / credentials |
| Text | `bedrock` | 15 | Docs / credentials |
| Text | `google` | 24 | Docs / credentials |
| Text | `deepseek` | 4 | Docs / credentials |
| Text | `xai` | 15 | Docs / credentials |
| Text | `xiaomi-mimo` | 3 | Docs / credentials |
| Text | `meta-model-api` | 7 | Docs / credentials |
| Text | `minimax` | 10 | Docs / credentials |
| Text | `mistral` | 15 | Docs / credentials |
| Text | `ollama` | 17 | Local |
| Text | `lmstudio` | 0 | Local |
| Text | `zhipu` | 17 | Docs / credentials |
| Text | `zhipu-intl` | 9 | Docs / credentials |
| Text | `moonshot` | 22 | Docs / credentials |
| Text | `siliconflow` | 75 | Docs / credentials |
| Text | `qwen-token-plan-cn` | 15 | Docs / credentials |
| Text | `qwen-token-plan-global` | 15 | Docs / credentials |
| Text | `qwen-cloud-intl` | 19 | Docs / credentials |
| Text | `alibaba-model-studio` | 59 | Docs / credentials |
| Text | `doubao` | 29 | Credentials / retained |
| Text | `yi` | 4 | HTTP 410; existing retirements retained |
| Text | `baichuan` | 10 | Credentials / retained |
| Image | `openai` | 8 | Docs / credentials |
| Image | `xai` | 2 | Docs / credentials |
| Image | `google-gemini` | 5 | Docs / credentials |
| Image | `qwen-dashscope-cn` | 11 | Docs / credentials |
| Image | `qwen-dashscope-intl` | 11 | Docs / credentials |
| Image | `zhipu-cogview` | 2 | Credentials / retained |
| Image | `openrouter-images` | 70 | Public API |
| Image | `custom-openai-images` | 0 | Custom |
| Image | `meta-model-api` | 1 | Docs / credentials |
| Embedding | `openai` | 2 | Docs / credentials |
| Embedding | `alibaba-model-studio-cn` | 3 | Docs / credentials |
| Embedding | `jina` | 16 | Public API |
| Embedding | `mistral` | 2 | Docs / credentials |
| Embedding | `siliconflow-cn` | 8 | Docs / credentials |
| Embedding | `siliconflow-international` | 3 | Docs / credentials |
| Embedding | `zhipu` | 2 | Docs / credentials |
| Embedding | `baidu-qianfan` | 1 | Credentials / retained |
| Embedding | `voyage` | 6 | Docs / credentials |
| Embedding | `cohere` | 5 | Docs / credentials |
| Embedding | `google-gemini` | 2 | Docs / credentials |
| Embedding | `openrouter` | 40 | Public API |
| Embedding | `ollama` | 5 | Local |
| Embedding | `lmstudio` | 0 | Local |
| Embedding | `custom` | 0 | Custom |
| Speech to text | `local-whisper` | 1 | Local |
| Speech to text | `openai` | 4 | Docs / credentials |
| Speech to text | `openai-live` | 1 | Credentials / retained |
| Speech to text | `groq` | 2 | Docs / credentials |
| Speech to text | `alibaba-qwen-audio-streaming` | 3 | Docs / credentials |
| Speech to text | `alibaba-qwen-audio` | 2 | Docs / credentials |
| Speech to text | `alibaba-qwen-asr` | 2 | Docs / credentials |
| Speech to text | `alibaba-qwen-realtime` | 2 | Docs / credentials |
| Speech to text | `siliconflow` | 2 | Docs / credentials |
| Speech to text | `sherpa-sense-voice` | 1 | Local |
| Speech to text | `sherpa-zipformer` | 1 | Local |
| Speech to text | `custom-openai` | 1 | Custom |
| Speech to text | `google-transcribe` | 1 | Docs / credentials |
| Speech to text | `meta-transcribe` | 1 | Credentials / retained |
| Speech to text | `google-transcribe-live` | 1 | Docs / credentials |
| Speech to text | `mistral-realtime` | 1 | Docs / credentials |
| Speech to text | `mistral` | 2 | Docs / credentials |
| Text to speech | `openai` | 4 | Docs / credentials |
| Text to speech | `groq` | 2 | Docs / credentials |
| Text to speech | `elevenlabs` | 7 | Docs / credentials |
| Text to speech | `azure-speech` | 1 | Credentials / retained |
| Text to speech | `minimax` | 2 | Docs / credentials |
| Text to speech | `qwen-audio-generation` | 1 | Docs / credentials |
| Text to speech | `dashscope-cosyvoice` | 8 | Docs / credentials |
| Text to speech | `sherpa-onnx` | 3 | Local |
| Text to speech | `siliconflow` | 2 | Docs / credentials |
| Text to speech | `google-gemini` | 2 | Docs / credentials |
| Text to speech | `mistral-realtime` | 1 | Docs / credentials |
| Text to speech | `mistral` | 1 | Docs / credentials |
| Decision | `typesafe` | 4 | Docs / credentials |
| Decision | `openai` | 1 | Docs / credentials |
| Decision | `openrouter` | 45 | Public API |
| Decision | `siliconflow-cn` | 2 | Docs / credentials |
| Decision | `siliconflow-international` | 1 | Docs / credentials |
| Decision | `alibaba-model-studio-cn` | 2 | Docs / credentials |
| Decision | `alibaba-model-studio-intl` | 2 | Docs / credentials |
| Video | `minimax-global` | 4 | Docs / credentials |
| Video | `minimax-hailuo-global` | 3 | Docs / credentials |
| Video | `runway-global` | 26 | Docs / credentials |
| Video | `bytedance-direct-watchlist` | 1 | Watchlist; unverified direct video adapter |
| Video | `openrouter-video` | 33 | Public API |

## Keeping hosted catalogs current

Use the existing [catalog audit](../../scripts/model-catalog-audit.mjs) and [scheduled drift workflow](../../.github/workflows/model-catalog-drift.yml). Public discovery must compare the full supported surface inventory and equivalent advertised metadata, not only a manually chosen list of prominent IDs. Authenticated discovery remains credential-scoped; set catalog-only credentials to extend coverage without running inference. The report lists every unprobed endpoint, so missing access cannot silently count as full verification.

An unavailable candidate is not an automatic retirement. Apply a removed tombstone only with endpoint-specific retirement evidence; retain future shutdowns as deprecated and preserve saved model identity. A free route must never silently migrate to a paid route. Refresh aggregator metadata even when the underlying native model has already been updated. Review contradictory marketing/API inventories before enabling a new surface.

The audit reuses the existing schedule and fixtures. No extra CI job, model daemon, or parallel provider runtime is needed. See [source references](MODEL_CATALOG_AND_TPS_REFERENCES.md) for the upstream implementation research.
