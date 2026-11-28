# Model catalog evidence: November 27, 2026

[Development documentation](README.md) · [Audit reference](MODEL_CATALOG_AUDIT.md)

Compared master `3465dc74d2c4c423c03bfd9619fe6244f6e8f9da` and prior draft
`6c6112f2af8f6ea27ea6903a69ee77e4a549bc0a` in [PR #604](https://github.com/MLGBJDLW/Nexa/pull/604).
No credentialed requests, paid inference, installs, auto-switches, index
migrations, or provider/connector configuration changes.

| Exact route or surface | November 27 observation and action |
| --- | --- |
| MAI-Image [2.5](https://openrouter.ai/microsoft/mai-image-2.5) and [2.5 Pro](https://openrouter.ai/microsoft/mai-image-2.5-pro) | Exact pages advertise November 27 retirement, without an hour; both [2.5](https://openrouter.ai/api/v1/images/models/microsoft/mai-image-2.5/endpoints) and [2.5 Pro](https://openrouter.ai/api/v1/images/models/microsoft/mai-image-2.5-pro/endpoints) still showed an Azure listing and no lifecycle metadata in their endpoint JSON. Deprecated, not removed; 2.7 and 2.7 Flash remain optional, no replacement of saved selections. [Recraft free](https://openrouter.ai/recraft/recraft-v4.1-flash-v2%3Afree) remains scoped to November 28; not the paid sibling. |
| [LingSante 3.1 Air VL](https://openrouter.ai/api/v1/models/inclusionai/lingsante-3.1-air-vl/endpoints) | Exact OpenRouter chat row: DeepInfra listing, text/image input, text output, 131,072 context and 32,768 output; tools and structured output advertised. Non-default, no validation of clinical use, no inferred reasoning controls. |
| [LingDT Decision Fast 1.0 free](https://openrouter.ai/api/v1/models/antdigital/lingdt-decision-fast-1.0%3Afree/endpoints) | Exact Ant Digital-backed OpenRouter Decisions listing: text in, decisions out, 32,768 context. Non-default; not available via the chat picker just because it's in the aggregator. No paid alternate or account eligibility inferred. |
| [GLM 4.6V](https://openrouter.ai/api/v1/models/z-ai/glm-4.6v/endpoints) | Current Z.AI listing and generic OpenRouter text row both omit structured output and response format while listing tools. Remove the unsupported scoped claims; keep modalities, defaults and native Z.AI unchanged. |
| [Kimi K2 0711](https://openrouter.ai/api/v1/models/moonshotai/kimi-k2/endpoints), [K2 0905](https://openrouter.ai/api/v1/models/moonshotai/kimi-k2-0905/endpoints) and [Pareto Latest](https://openrouter.ai/api/v1/models/~unbiased/pareto-latest/endpoints) | Two historical Kimi IDs reappeared with one Novita listing each; report for review, no account inference or automatic reactivation of tombstones. Pareto's rolling alias stays listed with no exact backing, even though structured expiration is now null; no permanent removal inferred. |
| Google Gemini Developer API [2.5 Pro](https://aistudio.google.com/docs/deprecations#gemini-2.5-pro-models) and [2.5 Flash](https://aistudio.google.com/docs/deprecations#gemini-2.5-flash-models) | Google announces December 1 reroutes to 3.8 Flash and 3.5 Flash-Lite respectively, at the successor's pricing. Nexa already shows correct dates and replacement suggestions. Serialized local matrix covers old numeric budget vs explicit successor thinking level and old temperature-only vs successor omission. Actual post-cutoff alias translation is unknown. Keep Vertex, OpenRouter, custom proxies and 2.5 Flash-Lite distinct; no saved route rewrite. |
| [DeepInfra status](https://status.deepinfra.com/) | Indexed first-party notice schedules preview redirect `tencent/Hy4-preview` → `tencent/Hy4` November 27 00:49 UTC. [New exact model API](https://deepinfra.com/tencent/Hy4/api) is documented. No inference or redirect behavior verified, and no dedicated Nexa DeepInfra preset; not evidence for SiliconFlow/OpenRouter. |
| [OpenRouter speech inventory](https://openrouter.ai/api/v1/audio/speech/models) | Anonymous GET returned 39 exact speech models. Keep the voice and response format enums by model and individually by endpoint. Examples: Google Gemini 2.5 Flash TTS, 30 voices and PCM/WAV; paid HeyGen Voice Turbo, 10 voices and PCM. Unknown fields are not supported controls. No runtime adapter, music, free-HeyGen lifecycle, or account capability inferred. |
| [OpenRouter transcription inventory](https://openrouter.ai/api/v1/models?output_modalities=transcription) | Anonymous GET returned 30 rows; all independently declare audio input, transcription output, and the transcription wire operation listed in the [official guide](https://openrouter.ai/blog/tutorials/transcription-on-openrouter/). Inventory only; no runtime, credential or generation claim. |

## Coverage

At 02:09 UTC the command validated 96 preset endpoints and 1,562 model
projections with no static errors. The new speech and transcription sections are separately
reported, not counted as an implemented or verified Nexa endpoint. Coverage
of the existing preset endpoints:

| Surface | Public success | Credential needed | No implemented operation inventory | Discovery unsupported |
| --- | --- | --- | --- | --- |
| Text | OpenRouter (477) | 20 | 0 | 6 |
| Image | OpenRouter (71) | 0 | 8 | 0 |
| Embedding | OpenRouter (40), Jina (30) | 2 | 13 | 0 |
| ASR | None (see discovery-only OpenRouter above) | 0 | 18 | 0 |
| TTS | None (see discovery-only OpenRouter above) | 0 | 13 | 0 |
| Decisions | OpenRouter (44) | 0 | 6 | 0 |
| Video | OpenRouter (30) | 0 | 4 | 0 |

The generated `report.json` and `report.md` enumerate exact endpoint IDs
with each reason and expose nonauthoritative inventory drift for review.
Provider regions and credentials are not pooled; in particular Alibaba
Beijing, Singapore and Token Plan coverage is not established by a
different platform's catalog. Provider quotas and account-specific
availability were not checked. The [public ACP registry](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json)
returned 46 entries; all 44 watched IDs matched and 29 versions differed.
Local/custom MCP/ACP tooling, executables and login-bound subscription
model catalogs were not installed, switched or audited as user inventory.
