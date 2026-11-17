# Model catalog evidence: 2026-11-17

Base: `0a609d24beed2353795cbc8ca5691a8145fe9fa0` (master). PR #564 is merged; #565/#566 are release-only. Rechecked master and recent PRs before preparing this change. No default, saved configuration, embedding index, price, or paid-route migration is performed.

## Implemented evidence

- [OpenRouter text inventory](https://openrouter.ai/api/v1/models): Hy4 Flash added; exact returned context/output limits and supported parameters refreshed. No inferred prices or reasoning effort controls.
- [Decisions inventory](https://openrouter.ai/api/v1/models?output_modalities=decisions&endpoints=/api/alpha/decisions): Liquid D1.2/Ultra added on Decisions only; HeyGen models advertise video input.
- [Video inventory](https://openrouter.ai/api/v1/videos/models): Seedance 2.5 Flash added with text/image/first-last-frame only, explicit 4–30 second options. Auto-duration sentinel and unverified video-input contracts omitted.
- [OpenAI deprecations](https://developers.openai.com/api/docs/deprecations): Nov 10 notice deprecates native GPT Image 2 with shutdown February 10, 2027. Mark deprecated, not removed; do not propagate to independently routed OpenRouter endpoint or force a replacement.
- Qwen Token Plan [CN](https://help.aliyun.com/zh/model-studio/token-plan-personal-overview) / [Singapore](https://www.alibabacloud.com/help/en/model-studio/token-plan-personal-overview), [release notes](https://www.alibabacloud.com/help/en/model-studio/token-plan-product-updates): formal qwen4-max on personal/team plans. Add opt-in account/region-scoped IDs; retain preview until its existing November 26 UTC retirement boundary. Do not infer limits from a different regional plan.
- [Cohere Embed](https://docs.cohere.com/docs/cohere-embed): fill two existing light-v3 catalog omissions at fixed 384 dimensions, not new-release claims. Existing default and other dimensions unchanged; generic native adapter already supports these IDs.

## Coverage and limitations

Inventory: 92 endpoint projections across text (27), image (9), embedding (15), STT (17), TTS (12), Decisions (7), video (5); plus 46 external-agent entries and 2 subscription runtimes. Public API audit succeeded at **6/92**, not full-provider verification. No authenticated inference or account entitlement test was performed. 21 endpoints lack configured credentials; 59 do not expose a supported operation inventory in this auditor; 6 do not support configured discovery. Those reasons describe the audit tool, not evidence of model unavailability.

The table below is the exhaustive endpoint audit coverage. Regional endpoint IDs stay separate; account, project and deployment-specific results remain runtime-authoritative. Public documentation review supplements but does not convert skipped API probes into verified account inventories.

| Endpoint | Inventory check | Details |
|---|---|---|
| `text:github-copilot-api` | skipped | discovery_not_supported |
| `text:openai` | skipped | credential_not_configured |
| `text:openrouter` | ok | 470 public records |
| `text:anthropic` | skipped | credential_not_configured |
| `text:vertex` | skipped | discovery_not_supported |
| `text:vertex-anthropic` | skipped | discovery_not_supported |
| `text:bedrock` | skipped | discovery_not_supported |
| `text:google` | skipped | credential_not_configured |
| `text:deepseek` | skipped | credential_not_configured |
| `text:xai` | skipped | credential_not_configured |
| `text:xiaomi-mimo` | skipped | credential_not_configured |
| `text:meta-model-api` | skipped | credential_not_configured |
| `text:minimax` | skipped | credential_not_configured |
| `text:mistral` | skipped | credential_not_configured |
| `text:ollama` | skipped | discovery_not_supported |
| `text:lmstudio` | skipped | discovery_not_supported |
| `text:zhipu` | skipped | credential_not_configured |
| `text:zhipu-intl` | skipped | credential_not_configured |
| `text:moonshot` | skipped | credential_not_configured |
| `text:siliconflow` | skipped | credential_not_configured |
| `text:qwen-token-plan-cn` | skipped | credential_not_configured |
| `text:qwen-token-plan-global` | skipped | credential_not_configured |
| `text:qwen-cloud-intl` | skipped | credential_not_configured |
| `text:alibaba-model-studio` | skipped | credential_not_configured |
| `text:doubao` | skipped | credential_not_configured |
| `text:yi` | skipped | credential_not_configured |
| `text:baichuan` | skipped | credential_not_configured |
| `image:openai` | skipped | operation_catalog_not_supported |
| `image:xai` | skipped | operation_catalog_not_supported |
| `image:google-gemini` | skipped | operation_catalog_not_supported |
| `image:qwen-dashscope-cn` | skipped | operation_catalog_not_supported |
| `image:qwen-dashscope-intl` | skipped | operation_catalog_not_supported |
| `image:zhipu-cogview` | skipped | operation_catalog_not_supported |
| `image:openrouter-images` | ok | 68 public records |
| `image:custom-openai-images` | skipped | operation_catalog_not_supported |
| `image:meta-model-api` | skipped | operation_catalog_not_supported |
| `embedding:openai` | skipped | operation_catalog_not_supported |
| `embedding:alibaba-model-studio-cn` | skipped | operation_catalog_not_supported |
| `embedding:jina` | ok | 30 public records |
| `embedding:mistral` | skipped | operation_catalog_not_supported |
| `embedding:siliconflow-cn` | skipped | operation_catalog_not_supported |
| `embedding:siliconflow-international` | skipped | operation_catalog_not_supported |
| `embedding:zhipu` | skipped | operation_catalog_not_supported |
| `embedding:baidu-qianfan` | skipped | operation_catalog_not_supported |
| `embedding:voyage` | skipped | operation_catalog_not_supported |
| `embedding:cohere` | skipped | operation_catalog_not_supported |
| `embedding:google-gemini` | skipped | credential_not_configured |
| `embedding:openrouter` | ok | 38 public records |
| `embedding:ollama` | skipped | operation_catalog_not_supported |
| `embedding:lmstudio` | skipped | operation_catalog_not_supported |
| `embedding:custom` | skipped | operation_catalog_not_supported |
| `speech_to_text:local-whisper` | skipped | operation_catalog_not_supported |
| `speech_to_text:openai` | skipped | operation_catalog_not_supported |
| `speech_to_text:openai-live` | skipped | operation_catalog_not_supported |
| `speech_to_text:groq` | skipped | operation_catalog_not_supported |
| `speech_to_text:alibaba-qwen-audio-streaming` | skipped | operation_catalog_not_supported |
| `speech_to_text:alibaba-qwen-audio` | skipped | operation_catalog_not_supported |
| `speech_to_text:alibaba-qwen-asr` | skipped | operation_catalog_not_supported |
| `speech_to_text:alibaba-qwen-realtime` | skipped | operation_catalog_not_supported |
| `speech_to_text:siliconflow` | skipped | operation_catalog_not_supported |
| `speech_to_text:sherpa-sense-voice` | skipped | operation_catalog_not_supported |
| `speech_to_text:sherpa-zipformer` | skipped | operation_catalog_not_supported |
| `speech_to_text:custom-openai` | skipped | operation_catalog_not_supported |
| `speech_to_text:google-transcribe` | skipped | operation_catalog_not_supported |
| `speech_to_text:meta-transcribe` | skipped | operation_catalog_not_supported |
| `speech_to_text:google-transcribe-live` | skipped | operation_catalog_not_supported |
| `speech_to_text:mistral-realtime` | skipped | operation_catalog_not_supported |
| `speech_to_text:mistral` | skipped | operation_catalog_not_supported |
| `text_to_speech:openai` | skipped | operation_catalog_not_supported |
| `text_to_speech:groq` | skipped | operation_catalog_not_supported |
| `text_to_speech:elevenlabs` | skipped | operation_catalog_not_supported |
| `text_to_speech:azure-speech` | skipped | operation_catalog_not_supported |
| `text_to_speech:minimax` | skipped | operation_catalog_not_supported |
| `text_to_speech:qwen-audio-generation` | skipped | operation_catalog_not_supported |
| `text_to_speech:dashscope-cosyvoice` | skipped | operation_catalog_not_supported |
| `text_to_speech:sherpa-onnx` | skipped | operation_catalog_not_supported |
| `text_to_speech:siliconflow` | skipped | operation_catalog_not_supported |
| `text_to_speech:google-gemini` | skipped | operation_catalog_not_supported |
| `text_to_speech:mistral-realtime` | skipped | operation_catalog_not_supported |
| `text_to_speech:mistral` | skipped | operation_catalog_not_supported |
| `decision:typesafe` | skipped | operation_catalog_not_supported |
| `decision:openai` | skipped | operation_catalog_not_supported |
| `decision:openrouter` | ok | 42 public records |
| `decision:siliconflow-cn` | skipped | operation_catalog_not_supported |
| `decision:siliconflow-international` | skipped | operation_catalog_not_supported |
| `decision:alibaba-model-studio-cn` | skipped | operation_catalog_not_supported |
| `decision:alibaba-model-studio-intl` | skipped | operation_catalog_not_supported |
| `video:minimax-global` | skipped | operation_catalog_not_supported |
| `video:minimax-hailuo-global` | skipped | operation_catalog_not_supported |
| `video:runway-global` | skipped | operation_catalog_not_supported |
| `video:bytedance-direct-watchlist` | skipped | operation_catalog_not_supported |
| `video:openrouter-video` | ok | 33 public records |

### Supplemental official-document review and unresolved work

- OpenAI, Anthropic, Google and Mistral release/lifecycle notices reviewed. Existing Hydrogen GA, Mistral Large 4 and Sonnet 4.5 future retirement already represented. This was document review, not account API validation.
- SiliconFlow [CN release notes](https://docs.siliconflow.cn/docs/release-notes/overview) announce seven November 16 removals; without explicit completion evidence, existing deprecated state is retained. [International release notes](https://docs.siliconflow.com/en/release-notes/overview) do not establish equivalent international retirement. No regional propagation.
- [Runway models](https://docs.dev.runwayml.com/guides/models/) and [OpenAPI](https://docs.dev.runwayml.com/openapi.json) confirm missing kling_4_flash and gemini_omni_flash_1.2. Deferred: operation-specific ratio, duration, videoUri and mode contracts differ from existing templates; needs dedicated fixture/adapter verification. act_two is a separate performance-control operation, not an ordinary video preset.
- [DeepSeek](https://api-docs.deepseek.com/quick_start/pricing/), [Meta](https://dev.meta.ai/docs/models), [MiniMax](https://platform.minimax.io/docs/guides/models-intro), [Zhipu CN](https://docs.bigmodel.cn/cn/guide/start/model-overview), [Z.ai](https://docs.z.ai/guides/overview/overview), [Kimi](https://platform.kimi.ai/docs/models), [Voyage](https://docs.voyageai.com/docs/embeddings), [Groq](https://console.groq.com/docs/models), [ElevenLabs](https://elevenlabs.io/docs/overview/models), [Azure voices](https://learn.microsoft.com/en-us/azure/ai-services/speech-service/language-support?tabs=tts) reviewed. Current major families represented. MiniMax older speech families need exact endpoint verification before additions. Kimi older models remain available to existing accounts and must not be removed early. Azure voice availability remains regional/runtime-dependent.
- [MiMo Pro](https://mimo.mi.com/models/en-US/mimo-v2.6-pro) verified; other variants incomplete. [Baidu embedding](https://cloud.baidu.com/doc/qianfan-api/s/qmwbl45cf) accessible, other families lack verified service IDs/dimensions in this pass. [Yi](https://platform.lingyiwanwu.com/assets/) and [Baichuan](https://platform.baichuan-ai.com/docs-v2/api) lacked readable complete contracts. Vertex, Bedrock and direct Doubao account/region inventories are unverified. Other speech/multimodal endpoints in the table were not individually verified through authenticated lists.
- [xAI migration notice](https://docs.x.ai/developers/migration/imagine-image-quality-nov-2) affects aliases not present locally; no broader image 1.0 retirement inferred.
- OpenRouter missing `nvidia/llama-nemotron-embed-vl-1b-v2:free` is retained: list absence alone is not retirement. Final online report therefore still reports drift for this candidate. No destructive tombstones added.
- ACP registry (46 entries including custom) and Copilot/Codex subscription runtimes were inspected as configuration only; no agents installed or authorized. Ollama/LM Studio, local Whisper/Sherpa and custom endpoints require actual user runtime discovery. Upstream releases do not prove local installation.

## Verification

- Static catalog audit: zero errors.
- Audit fixture tests and external-agent registry contract: passed.
- All provider-preset and image-capability Node contract tests: 50/50 passed (includes 7 new November 17 tests).
- Dependency install used `--ignore-scripts`; initial default npm cache was inaccessible, retry with writable /tmp cache succeeded. Lockfile unchanged.
- Rust tests/build not run: cargo unavailable. No real inference, pricing/entitlement validation or browser/UI test. CI is pending a pushed commit and must not be reported as passed.
