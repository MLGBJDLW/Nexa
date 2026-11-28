# Model catalog evidence: November 26, 2026

[Development documentation](README.md) · [Audit reference](MODEL_CATALOG_AUDIT.md)

Compared master `3465dc74d2c4c423c03bfd9619fe6244f6e8f9da` after
[PR #601](https://github.com/MLGBJDLW/Nexa/pull/601). The catalog and
public route checks are not account or generation checks. No credentialed
provider requests, paid calls, model or agent installation, automatic
model switches, index migration, or changes to privacy settings were made.

| Exact route | November 26 finding | Scope |
| --- | --- | --- |
| [OpenRouter Solar Pro 4 Contributor](https://openrouter.ai/upstage/solar-pro-4-contributor) | Upstage may use prompts and outputs to improve its models. Retain the warning in the bundled, live and cached routes, above-the-fold in manual choices. | Only `upstage/solar-pro-4-contributor` on the OpenRouter preset. No default, configuration, or upstream policy change. |
| OpenRouter Pareto [26.10](https://openrouter.ai/unbiased/pareto-26.10), [26.10 Preview](https://openrouter.ai/unbiased/pareto-26.10-preview), [26.9](https://openrouter.ai/unbiased/pareto-26.9) | Each exact model card says no providers since November 25 and each exact endpoints API has no routes. | Retain fixed IDs as tombstones without a replacement. The rolling alias remains listed with no current backing; do not permanently retire it or rebind it. |
| OpenRouter Gemini [2.5 Flash](https://openrouter.ai/google/gemini-2.5-flash) and [2.5 Pro](https://openrouter.ai/google/gemini-2.5-pro) | OpenRouter cards and public model list independently announce December 1 retirement. | Upcoming and deprecated only, no replacement inferred from the separate Gemini Developer API redirects. |
| [OpenRouter TNG DeepSeek R1T2 Chimera](https://openrouter.ai/api/v1/models/tngtech/deepseek-r1t2-chimera/endpoints) | Text model listed with one active Featherless route, 131,072 context, 32,768 output; no tool parameter advertised. | Non-default known text route with structured output; no native DeepSeek, account or other provider claims. |
| Alibaba Qwen4 Max Preview [Beijing Token Plan](https://docs.bailian.console.aliyun.com/zh/model-studio/token-plan-product-updates) and [Singapore Token Plan](https://docs.modelstudio.console.alibabacloud.com/en/model-studio/token-plan-product-updates) | Starting November 25 at 10:00 UTC+8, the vendor routes the original ID to `qwen4-max` at standard rates, not the preview discount. | Keep the prior ID and alias callable in Nexa. Deprecation and replacement are guidance only; no request rewrite or default change. |
| Alibaba [Decision Model](https://docs.modelstudio.console.alibabacloud.com/en/model-studio/decision-model-stable) | Stable `decision-model` accepts text and image, with an official state maximum of 268,288 tokens. It uses the [same TypeSafe API](https://docs.modelstudio.console.alibabacloud.com/en/model-studio/decision-model-api) as the preview. | Only existing Beijing and Singapore Decisions workspace adapters; opt-in, account/workspace eligibility unverified. Preview, empty default and exact workspace rules remain. |

Already handled in #601: OpenRouter Liquid D1.1 and five video retirements,
Beijing Ark's three exact November 24 rows, and GLM-5.4 Coding Plan
guidance. Ark Seed 2.0 Pro's distinct January extension remains, and the
Ark Personal Agent Plan is not an existing native endpoint. GLM-5.4
standard API, MiniMax native image, DeepSeek independent ASR/TTS, and
Alibaba realtime voice were not inferred from specialized products.

## Coverage

`npm run catalog:audit -- --live-public` validates 96 endpoint projections.
Six public discovery calls went through: OpenRouter text (474), image (71),
embedding (40), Decisions (43), and video (30); Jina embedding (30).
Of the other 90 endpoints, 22 lack credentials, 62 have no implemented
operation inventory, and six do not support this audit's discovery. These
are not claims of regional or tenant coverage. No public ASR or TTS list
is part of this audit. Runway's public operation schema still covers the
current video rows but does not demonstrate account entitlements. The
public ACP registry had 46 records, with all 44 shipped registry pins
matching an ID and 28 observed version differences; runtimes, pins,
subscription accounts and custom/local installations were not altered.

Contract tests distinguish live and stale-cache precedence, authentic
endpoint boundaries, both Token Plan regions, chooser wrapping, exact
retirement scope, and the existing Alibaba System One request contract.
