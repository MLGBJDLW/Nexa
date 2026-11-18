# Model catalog evidence: 2026-11-18

[Development documentation](README.md)

Base master: `406c38f067a52a627ac73834d6d3289fe78c7fa9` (0.19.14). November 17 PRs #567/#569 and release PRs are already merged; no open PR existed at the initial check. This audit does not repeat those patches.

## Verified changes

- [OpenAI release notes](https://developers.openai.com/api/docs/changelog), [GPT-6.2 Astra](https://developers.openai.com/api/docs/models/gpt-6.2-astra), [GPT-6.2 Sol](https://developers.openai.com/api/docs/models/gpt-6.2-sol): native exact IDs, 1,050,000 context / 128,000 output, text/image input, structured output and Responses tools/web search. Extend exact public-OpenAI Responses routing and always-on reasoning; none/minimal map to low and temperature is omitted. Native catalog exposes only verified low/medium/high with medium default, not an inferred complete effort set. Private compatible endpoints and Router `-pro` aliases do not inherit native routing.
- [OpenRouter text inventory](https://openrouter.ai/api/v1/models): four exact GPT-6.2 Astra/Sol and Pro aliases, with independently advertised Chat Completions tools and low/medium/high/xhigh/max efforts. Pro aliases stay opt-in; no saved/default model is switched to them. Refresh five exact max-output limits from top-provider metadata. Prices are not guessed or copied.
- The same public inventory explicitly announces future expiration: `thinkingmachines/inkling-2-small:free` November 19 and `mistralai/mistral-medium-3-5` January 31, 2027. Keep both deprecated, not removed, scoped to OpenRouter only.
- [OpenRouter image inventory](https://openrouter.ai/api/v1/images/models): MAI-Image-2.7 / Flash with returned aspect ratios, transparent/opaque background and up to five references. [Decisions inventory](https://openrouter.ai/api/v1/models?output_modalities=decisions&endpoints=/api/alpha/decisions): Microsoft-Decision-1.1 text/image input, 32,768 context; Liquid D1 Ultra / D1.1 future November 24 expiration retained as deprecated.
- [OpenRouter video inventory](https://openrouter.ai/api/v1/videos/models): HeyGen Avatar 6 / Turbo recorded as **contract_pending and unselectable**, with no invented text/image operation. Their audio-driven avatar contract lacks the durations/frame/ratio information needed by the current adapter. This is an inventory record, not enabled generation.
- [Alibaba Macaron](https://www.alibabacloud.com/help/en/model-studio/macaron-mindlab): three exact IDs added only to Singapore PAYG, account_enablement, not Token Plan or China. Conservative ID-only entries avoid unsupported reasoning/capability inference; actual entitlement remains account-specific.
- [Doubao deprecation notice](https://docs.volcengine.com/docs/ark/model-deprecation-notice?lang=zh): fill missing November 24 deprecationDate for already-deprecated `doubao-seed-2-0-pro-260215`; exact upstream EOS is 14:00 UTC+8. Preserve its existing replacement/default semantics; do not mark removed early.
- [Runway OpenAPI](https://docs.dev.runwayml.com/openapi.json), [developer directory](https://dev.runwayml.com/models): `hailuo3_highspeed` / `hailuo3_lite` fill discovered catalog gaps, not claimed November 18 releases. Enable only media-free text-to-video: prompt 1–7000 UTF-16 units; explicit 480P/768P; integer 4–15 seconds; six published ratios. Exclude adaptive, references, all input media, audio/seed controls and advanced operations until model-specific input rules are verified. Dated and hashed schema projections and positive/negative adapter tests accompany additions. Existing Kling/Gemini Runway text-only constraints remain unchanged.

## Actual inventory coverage

Dynamically enumerated 92 operation endpoints: text 27, image 9, embedding 15, STT 17, TTS 12, Decisions 7, video 5. Baseline 1,437 model projections, rising to 1,456 after the updates. Also enumerated 46 external-agent entries and two subscription runtime entries. These are not 140 remotely verified APIs.

Public audit **6/92 successful**, zero failed probes, not full-provider coverage. 22 credential_not_configured, 58 operation_catalog_not_supported and 6 discovery_not_supported. Final public comparison reports zero drift, which means only the six checked inventories match; unselectable Avatar records are not validated inference. No credentials requested, no account-specific inventory or paid inference executed.

| Endpoint / regional preset | Probe | Coverage detail |
|---|---|---|
| `text:github-copilot-api` | skipped | discovery_not_supported |
| `text:openai` | skipped | credential_not_configured |
| `text:openrouter` | ok | 478 public records |
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
| `image:openrouter-images` | ok | 70 public records |
| `image:custom-openai-images` | skipped | operation_catalog_not_supported |
| `image:meta-model-api` | skipped | operation_catalog_not_supported |
| `embedding:openai` | skipped | operation_catalog_not_supported |
| `embedding:alibaba-model-studio-cn` | skipped | operation_catalog_not_supported |
| `embedding:jina` | ok | 30 public records |
| `embedding:mistral` | skipped | operation_catalog_not_supported |
| `embedding:siliconflow-cn` | skipped | operation_catalog_not_supported |
| `embedding:siliconflow-international` | skipped | operation_catalog_not_supported |
| `embedding:zhipu` | skipped | operation_catalog_not_supported |
| `embedding:baidu-qianfan` | skipped | credential_not_configured |
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
| `decision:openrouter` | ok | 43 public records |
| `decision:siliconflow-cn` | skipped | operation_catalog_not_supported |
| `decision:siliconflow-international` | skipped | operation_catalog_not_supported |
| `decision:alibaba-model-studio-cn` | skipped | operation_catalog_not_supported |
| `decision:alibaba-model-studio-intl` | skipped | operation_catalog_not_supported |
| `video:minimax-global` | skipped | operation_catalog_not_supported |
| `video:minimax-hailuo-global` | skipped | operation_catalog_not_supported |
| `video:runway-global` | skipped | operation_catalog_not_supported |
| `video:bytedance-direct-watchlist` | skipped | operation_catalog_not_supported |
| `video:openrouter-video` | ok | 35 public records |

## Supplemental official-document review and remaining boundaries

- [Anthropic release notes](https://platform.claude.com/docs/en/release-notes/overview) Nov17 billing update is not a model release; [Sonnet4.5 Nov30 retirement](https://platform.claude.com/docs/en/about-claude/model-deprecations) remains future. [Google release notes](https://aistudio.google.com/docs/changelog) latest visible Nov12; [Mistral](https://docs.mistral.ai/resources/changelogs) latest visible Oct29. No further verified model edits.
- [SiliconFlow CN](https://docs.siliconflow.cn/docs/release-notes/overview) Nov16 rate-limiting update does not prove planned removals completed; preserve existing lifecycle states. [International](https://docs.siliconflow.com/en/release-notes/overview) latest visible June11, not synchronized to CN. Typed embedding/Decisions/audio inventories remain unverified without appropriate credentials/contracts.
- [Alibaba CN lifecycle index](https://help.aliyun.com/zh/model-studio/model-depreciation) and [international index](https://www.alibabacloud.com/help/en/model-studio/model-depreciation) reference a January19,2027 plan but its specific announcement content was unavailable; no IDs inferred. Token Plan formal Qwen4 Max already represented, region/account availability untested.
- [Vertex lifecycle](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/model-versions): gemini-3.6-flash Nov19 and 3.7-flash Jan28 already deprecated; no early removal or propagation to Developer API. [Vertex Claude](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/partner-models/claude): latest four families represented, older family contracts not exhaustively reviewed.
- [Bedrock legacy lifecycle](https://docs.aws.amazon.com/bedrock/latest/userguide/model-lifecycle-legacy.html): regional Nov13 older Claude IDs absent from current presets; cannot extrapolate to remaining regions. [AWS GPT6.2 release](https://aws.amazon.com/about-aws/whats-new/2026/11/openai-gpt-6-2-astra-sol-amazon-bedrock/) known, but exact model IDs, regional endpoints and inference profiles could not be verified in model cards. No guessed `global.openai.*` additions. Vertex/Bedrock deployments remain runtime/account-specific. Doubao full model list timed out; lifecycle notice was readable.
- Reviewed public docs for [DeepSeek](https://api-docs.deepseek.com/quick_start/pricing/), [xAI](https://docs.x.ai/developers/release-notes), [MiMo Pro](https://mimo.mi.com/models/en-US/mimo-v2.6-pro), [Meta](https://dev.meta.ai/docs/models), [MiniMax](https://platform.minimax.io/docs/guides/models-intro), [Zhipu CN](https://docs.bigmodel.cn/cn/guide/start/model-overview), [Z.ai](https://docs.z.ai/release-notes/new-released), [Kimi](https://platform.kimi.ai/docs/models), [Cohere](https://docs.cohere.com/docs/cohere-embed), [Voyage](https://docs.voyageai.com/docs/embeddings), [Groq](https://console.groq.com/docs/models), [ElevenLabs](https://elevenlabs.io/docs/overview/models) and [Azure Speech](https://learn.microsoft.com/en-us/azure/ai-services/speech-service/language-support?tabs=tts). No other verified edits beyond the already-merged November17 updates. Kimi's older-account models must not be prematurely removed. MiMo Flash/ultraspeed not individually rechecked; Azure regional voices unverified. Jina's typed public embeddings checked separately from rerank/OCR/Reader records.
- The [Yi platform](https://platform.lingyiwanwu.com/) confirms its September 3 shutdown. Its four public models already have retirement records. [Baichuan Chat Completions](https://platform.baichuan-ai.com/docs-v2/api) and [pricing](https://platform.baichuan-ai.com/prices) confirm the medical and general-purpose families in the picker. Baichuan2-53B appears on pricing but has no current Chat Completions contract.
- [Qianfan v2 embeddings](https://cloud.baidu.com/doc/qianfan-api/s/qmwbl45cf), [BGE model cards](https://huggingface.co/BAAI/bge-large-en) and [Tao model configuration](https://huggingface.co/Amu/tao-8k/blob/main/config.json) confirm bge-large-zh, bge-large-en and tao-8k as 1,024-dimensional text models. Tao accepts one text per request. The existing 384-dimensional embedding-v1 remains the recommendation. The [typed models API](https://cloud.baidu.com/doc/qianfan-api/s/Amwbl43sp) can audit account-enabled embedding IDs with a scoped credential. Qwen3 model labels in the input-limit table alone do not establish the deployed wire IDs.
- The [official ACP registry](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json) lists 46 entries. All 44 supported registry agents are represented alongside Hermes and the custom launcher. Seventeen package or binary release pins advanced. Package metadata and publisher release metadata were checked without installing agents. Devin supplies no registry digest, consistent with the previous release. Copilot and Codex expose the authenticated account inventory. [Copilot lists GPT-6.2 Astra and Sol](https://docs.github.com/en/copilot/reference/ai-models/supported-models). Local Ollama, LM Studio, Whisper/Sherpa and user-defined endpoints were configuration-enumerated only.

## Verification

The static audit validated 92 endpoints and 1,456 model projections. All six public probes completed without reported drift. The provider contract tests, registry importer fixtures, catalog audit fixtures, documentation checks, Rust formatting and Windows desktop frontend build passed locally. GitHub Actions records the Rust runtime tests, browser checks and platform builds. Account entitlements and paid inference were outside this verification.

## Follow-up after 0.19.15

The [OpenAI model comparison](https://developers.openai.com/api/docs/models) gives both GPT-6.2 models five reasoning efforts from low through max. Nexa now features Sol and Astra first and offers all five in the direct API picker. Sol is the recommendation for everyday use. The [Chat Latest model card](https://developers.openai.com/api/docs/models/chat-latest) also confirms a separate text and image chat alias.

The published Bedrock model cards give the US and Global inference profile IDs, Converse support, input modalities and token limits for [Astra](https://docs.aws.amazon.com/bedrock/latest/userguide/model-card-openai-gpt-6-2-astra.html) and [Sol](https://docs.aws.amazon.com/bedrock/latest/userguide/model-card-openai-gpt-6-2-sol.html). All four profiles require account enablement. The existing Bedrock adapter uses Converse on the regional Runtime endpoint.

Alibaba's October retirement table identifies six saved Beijing pay-as-you-go IDs that can now be hidden from new selections. The January notice closes new purchases of Qwen3.5 Plus on November 18 and gives existing users until January 19 to migrate to Qwen3.8 Flash. [October retirement table](https://www.alibabacloud.com/help/fr/model-studio/model-depreciation) and [January retirement notice](https://www.alibabacloud.com/notice/detail?_p_lc=1&id=2217).

The follow-up enumerated all 27 configured chat endpoints. OpenRouter's 478 public records updated Qwen3 Coder's structured output and supported parameters and GLM-5.3 Flash's maximum output. SiliconFlow China's public inventory still included the deprecated Qwen3 Coder route with normal status and all 55 curated chat IDs. The other endpoints were reviewed from public documentation, with account, region and locally installed model availability unverified. [OpenRouter catalog](https://openrouter.ai/api/v1/models) and [SiliconFlow inventory](https://www.siliconflow.cn/models).
