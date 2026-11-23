# Model catalog evidence: November 21, 2026

[Development documentation](README.md) · [Maintenance policy](MODEL_CATALOG_MAINTENANCE.md)

Baseline: `60b3169b40d7289d35bd891f699c96469ac17416`. No open PR
overlapped at the start of this audit. Discovery was public and unauthenticated;
no inference, credential use, agent installation, default changes, index
migration, account changes, merge or release was performed.

## Changes

- Beijing Ark: keep the four November 24 models deprecated and still selectable
  before the announced EOS at 14:00 China Standard Time (06:00 UTC). Add advisory
  links from Seed 2.0 Code Preview and GLM 5.1 to `doubao-seed-evolving` and from
  DeepSeek V4 Pro to `deepseek-v4-1-flash-261017`. Ark separately documents an
  Economy variant for the service's own residual endpoint replacement; Nexa
  does not implement that switch. Keep the already-correct Seed 2.0 Pro advice
  and do not assume Ark's automatic destination. [Ark lifecycle notice](https://docs.volcengine.com/docs/ark/model-deprecation-notice?lang=zh).
- Vertex Gemini 3.6 Flash: new or inactive projects have been blocked since
  November 19; existing active projects have access until January 28, 2027.
  Update the permanent shutdown date and the advertised replacement to Gemini
  4 Hydrogen for Vertex Gemini 3.6/3.7 Flash. Preserve the deprecated status,
  saved model IDs, and separate routes. Gemini Developer API still reroutes
  3.7 to 3.8, and its 3.6 date is not announced. Neither Vertex date applies to
  OpenRouter. [Vertex lifecycle](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/model-versions),
  [3.6 Flash](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/gemini/3-6-flash),
  [3.7 Flash](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/gemini/3-7-flash),
  [Gemini Developer API](https://aistudio.google.com/docs/deprecations).
- OpenRouter chat: add `radixark/daisyrose-27b`, text/image to text, context
  262144, maximum output 235929, tools and structured output. DeepInfra is
  listed as the backing endpoint; no reasoning presets or effort range are
  inferred. Correct both MiMo V2.6 standard routes to the published context
  1048576. For DeepSeek V4 Flash Vision Experimental use the catalog's 262144
  output fallback: the backing inventory corroborates the catalog's lower
  value and no listed endpoint supports the old 943717 cap.
  [Catalog](https://openrouter.ai/api/v1/models),
  [DaisyRose endpoint](https://openrouter.ai/api/v1/models/radixark/daisyrose-27b/endpoints),
  [DeepSeek Vision endpoint](https://openrouter.ai/api/v1/models/deepseek/deepseek-v4-flash-vision-exp/endpoints).
- Alibaba Model Studio Singapore PAYG: add opt-in, account-enabled
  `mindlab/macaron-v1.1-venti-pro`, text/image to text, context 1048576,
  output 131072, tools, structured output and cache capability. This is not a
  Chinese mainland or Token Plan route. The service recommends the
  workspace-specific OpenAI-compatible URL; use no global URL rewrite.
  Activation and authorization are account-dependent; no vendor-specific
  reasoning controls are promised by this entry.
  [Model](https://www.alibabacloud.com/help/en/model-studio/macaron-v1-1-venti-pro-by-mindlab),
  [Endpoint and access](https://www.alibabacloud.com/help/en/model-studio/macaron-mindlab).
- OpenRouter Decisions only: add `baseten/groundhog-1` (16384 context) and
  `baseten/bison-1` (32768); both take text/image and return decisions,
  compatible with the existing `/api/alpha/decisions` path. Neither is
  recommended or a chat model. Groundhog does not advertise in-region EU
  processing; do not infer residency for either route.
  [Decisions catalog](https://openrouter.ai/api/v1/models?output_modalities=decisions),
  [Groundhog](https://openrouter.ai/baseten/groundhog-1),
  [Bison](https://openrouter.ai/baseten/bison-1).
- Runway `seedance2_5_flash`: add an opt-in text-to-video route only,
  with nonempty prompt up to 15000 UTF-16 code units, integer duration 4–30 seconds,
  explicit 480p/720p ratios, optional seed and audio. The existing upload
  and reference capabilities are not assumed for this new preset. Keep
  image-to-video, video-to-video, reference media, 1080p, auto sizing and
  alternative containers unavailable until those integrations are tested.
  Provider pricing for prompt-only generation is 13 or 20 credits per output
  second by resolution; no request was submitted.
  [API schema](https://docs.dev.runwayml.com/openapi.json),
  [Models](https://docs.dev.runwayml.com/guides/models/),
  [Pricing](https://docs.dev.runwayml.com/guides/pricing/).

## Deferred with reasons

- Inkling 2 Mini has a [vendor model card](https://thinkingmachines.ai/model-card/inkling-2-mini/)
  and a public OpenRouter page, but its [endpoint response](https://openrouter.ai/api/v1/models/thinkingmachines/inkling-2-mini/endpoints)
  has no backing entries, and the runnable text inventory did not return it.
  Neither the open weights nor the listing prove a hosted route.
- [Hy4](https://github.com/Tencent-Hunyuan/Hy4) flagship weights do not add
  a new native Tencent connection. The [Tencent Cloud preview notice](https://intl.cloud.tencent.com/announce/detail/101604)
  concerns December 7 at 00:00 China Standard Time on its own services
  (December 6 at 16:00 UTC); [OpenRouter's preview](https://openrouter.ai/api/v1/models/tencent/hy4-preview/endpoints)
  has independent backing. [WeMM-Video](https://github.com/Tencent/WeMM-Video)
  is understanding with text output, not a live hosted ID, installed model or
  video-generation adapter.
- Perplexity contextual Embedding V2 [0.6B](https://openrouter.ai/perplexity/pplx-embed-v2-context-0.6b)
  and [9B](https://openrouter.ai/perplexity/pplx-embed-v2-context-9b) appear in
  the public inventory. Safe adoption needs endpoint-scoped query/document
  `input_type`. The current OpenRouter adapter sends flat strings; it cannot
  infer contextual document boundaries or offer image-only document support.
  Preserve existing embedding space identifiers and indices until a tested
  opt-in request contract handles them.
- [Runway's November 20 release](https://docs.dev.runwayml.com/changelog/)
  adds `minimax_h3_image` to its image operation; Nexa's Runway connector is
  for video. OpenRouter's advertised speech-generation models have no existing
  native OpenRouter TTS preset. Neither is a catalog-only enablement.
- HeyGen Decisions aggregate audio/context metadata conflicted with the
  [model endpoint](https://openrouter.ai/api/v1/models/heygen/heygen-decision-1/endpoints);
  no audio expansion. Disappearance of Liquid D1 and three text routes
  (`cohere/north-mini-code:free`, `tencent/hy3-preview`, `x-ai/grok-build-0.1`)
  was not treated as verified retirement. Liquid D1.1 keeps its separate
  November 24 date. Remaining limits/parameter drift is largely routing-specific:
  e.g. standard DeepSeek V4.1 Flash, MiniMax M2.5 and DeepSeek latest alias
  have provider spread or changing top-provider reports. MiniMax M2.7 returned
  one backing provider reporting status `-2`; a transient snapshot does not
  prove permanent removal of structured output. No unverified change to
  Mistral dates or paid replacement routing.

## Coverage and checks

The final credential-free online audit recorded 94 endpoints, 1531 model
projections, zero static errors; all six attempted public reads succeeded.
Text 27/1069, image 9/119, embedding 17/125, speech recognition 17/33,
speech synthesis 12/44, Decisions 7/62 and video 5/79 (endpoint/models).
Of the 88 unprobed endpoints, 22 require credentials, 60 lack an implemented
operation listing, and six have unsupported or runtime discovery. This is
not a claim that all upstream inventories were verified.

Copilot and ChatGPT/Codex subscription presets deliberately have empty
runtime inventories. The 46 ACP configurations were enumerated separately:
a configured version is not evidence of install, sign-in, or model access.
The [official registry](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json)
was readable through segmented public indexing; direct access returned 403.
The indexed snapshot showed 19 version differences among 44 tracked IDs.
Package/install source and binary checksum corroboration remained incomplete,
so no pins were modified.
Ollama, LM Studio, Whisper, Sherpa and custom connections are local or
deployment runtime state. Region-specific access was not tested with an
account, including Vertex project/region eligibility, Ark Beijing, Alibaba
Singapore/Beijing/Hong Kong, SiliconFlow China/international or Azure.
Past documentary checks are not carried forward as new live probes.

Catalog audit fixtures and the five registry/Runway contracts passed; all 39
focused provider regression cases passed. The five new cases are included in
the desktop frontend contract suite. No Rust toolchain was available locally.
Native and full frontend validation are left to exact-head CI.

The table below records the actual outcome of each automated endpoint,
not whether the provider itself was reachable, available or supported.

| Endpoint / scope | Discovery outcome |
| --- | --- |
| `text:github-copilot-api` | discovery_not_supported |
| `text:openai` | credential_not_configured |
| `text:openrouter` | public inventory: 477 |
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
| `embedding:openrouter` | public inventory: 40 |
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
| `decision:openrouter` | public inventory: 44 |
| `decision:siliconflow-cn` | operation_catalog_not_supported |
| `decision:siliconflow-international` | operation_catalog_not_supported |
| `decision:alibaba-model-studio-cn` | operation_catalog_not_supported |
| `decision:alibaba-model-studio-intl` | operation_catalog_not_supported |
| `video:minimax-global` | operation_catalog_not_supported |
| `video:minimax-hailuo-global` | operation_catalog_not_supported |
| `video:runway-global` | operation_catalog_not_supported |
| `video:bytedance-direct-watchlist` | operation_catalog_not_supported |
| `video:openrouter-video` | public inventory: 35 |
