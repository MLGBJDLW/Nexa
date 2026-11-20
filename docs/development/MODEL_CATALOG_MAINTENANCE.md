# Model catalog maintenance

[Development documentation](README.md) · [Provider guide](../guides/PROVIDERS_AND_MODELS.md)

The shared presets remain the reviewed fallback for desktop and mobile. Model
availability, a vendor's advertised capabilities, and working inference with a
particular account are separate evidence. Never retire a saved model merely
because a request failed or an account-scoped list omitted it.

## Daily drift monitoring

The existing [model catalog drift workflow](../../.github/workflows/model-catalog-drift.yml)
runs daily. It validates every preset and emits one coverage row per endpoint,
including aggregators independently of their upstream providers. The public
OpenRouter text, image, embedding, decision and video inventories and Jina's
embedding inventory work without credentials. New models are compared against
the whole relevant inventory; no release-specific required-ID allowlist hides
later additions. Batch-only and non-chat routes are excluded from streaming chat.

Run the same checks locally:

```sh
npm run catalog:audit:test
npm run catalog:audit
node scripts/model-catalog-audit.mjs --live-public
```

The report in `.artifacts/model-catalog-drift` identifies additions, unavailable
candidates, changed reasoning controls, supported parameters, capacities,
modalities and lifecycle announcements. It is a review proposal, not an automatic
preset rewrite. Follow the linked official model documentation before accepting
new operation semantics or retiring a route. A confirmed retirement should keep
an endpoint-scoped tombstone and replacement where known.

## Account-scoped catalogs

Endpoints without public discovery remain explicitly unverified in the report.
For local authenticated audits, set the credential environment variable named in
that endpoint's coverage row. Existing OpenAI, OpenRouter, DeepSeek and Alibaba
variables continue working. Other OpenAI-compatible endpoints use
`NEXA_<PRESET_ID_IN_UPPER_SNAKE_CASE>_CATALOG_API_KEY`; native Google and Anthropic
discovery use their own authentication and pagination. No inference is requested.
Google's model-list methods identify text generation and embedding operations;
image and speech endpoints are explicitly unprobed because `generateContent`
alone does not establish their output modality.
Other generic OpenAI-compatible `/models` endpoints are audited for text only.
Image, embedding, speech, decision and video inventories require an explicit
operation contract; credentials alone do not make an untyped model list valid
for those surfaces. Dedicated OpenRouter operation paths and Jina's explicitly
typed embedding entries retain their existing coverage.

For Actions, the optional `NEXA_MODEL_CATALOG_CREDENTIALS` repository secret is a
JSON object mapping exact endpoint IDs to account credentials. For example, its
keys can include `text:siliconflow`, `text:qwen-token-plan-global`,
`text:qwen-cloud-intl`, `text:alibaba-model-studio`, `text:doubao`,
`text:mistral`, or any other supported discoverable
endpoint. This avoids adding a workflow variable whenever a provider is added.
Separate endpoints keep separate entries; one upstream account does not verify
an aggregator's entitlement. Unsupported discovery contracts, local servers,
subscription catalogs and deployment-specific Bedrock/Vertex access remain
clearly marked for their native account/deployment verification.

Requests have a single pagination-and-body deadline, validate continuation
origins and reject incomplete or malformed inventories. Reports do not contain
credentials, request headers or raw response bodies. The workflow updates its
existing drift issue and retains report artifacts for seven days.

## Runtime discovery

OpenRouter's existing provider catalog hook now retains explicit model names,
tool/JSON capabilities, reasoning controls, input modalities and context/output
limits. A newly listed model can therefore expose advertised capabilities before
the next built-in release. Unknown fields do not invent capabilities or effort
levels, and omitted optional metadata preserves reviewed facts. Explicit removed
controls can withdraw previously advertised controls.

The cache uses a hash of endpoint and credential identity, coalesces requests per
account, retains the last successful inventory on transport failure, and reports
that failed refresh honestly. A five-minute refresh interval and failure cooldown
avoid repeating catalog requests for every model step. At most 32 account caches
are retained. The ten-second complete discovery deadline fits inside the existing
twelve-second cloud picker deadline; unrelated accounts do not share an I/O lock.

These mechanisms follow the official [OpenRouter Models API](https://openrouter.ai/docs/api_reference/models),
[Gemini model-list API](https://aistudio.google.com/api-reference/models), and
[Anthropic model-list API](https://platform.claude.com/docs/en/api/models/list).
See the [upstream research](MODEL_CATALOG_AND_TPS_REFERENCES.md) and
[catalog audit](MODEL_CATALOG_AUDIT.md) for the dated implementation evidence.

## November 20, 2026 verification

The scheduled audit covered all 94 endpoint definitions. Six published a public
inventory, 22 required credentials, 60 had no operation inventory and six used
runtime discovery or an unsupported inventory operation. A second local pass
read all five public OpenRouter inventories. Jina timed out locally, and its
public inventory responded through an independent connection. These probes used
model listings. They did not run paid inference.
[Full endpoint audit](MODEL_CATALOG_EVIDENCE_2026_11_20.md) · [Jina model list](https://api.jina.ai/v1/models)

At 08:51 UTC the OpenRouter inventory and per-model endpoint lists confirmed
that Llama 3.2 3B was still served after its published deadline was withdrawn.
Llama 3.2 1B and the free Inkling 2 Small route had both passed their deadlines
and had zero endpoints. Kimi K2.7 Code and Llama 3.2 3B no longer advertised
`min_p`. GLM 5.3 Flash advertised a 131,072-token maximum output.
[OpenRouter models](https://openrouter.ai/api/v1/models) · [Llama 3B endpoints](https://openrouter.ai/api/v1/models/meta-llama/llama-3.2-3b-instruct/endpoints) · [Llama 1B endpoints](https://openrouter.ai/api/v1/models/meta-llama/llama-3.2-1b-instruct/endpoints) · [Inkling endpoints](https://openrouter.ai/api/v1/models/thinkingmachines/inkling-2-small:free/endpoints)

Alibaba's Beijing and Singapore plan lists include Qwen4 Max and its preview
alias. Google-managed Claude lists Mythos 5.2 with account-gated access and a
Fast tier. A local authenticated Codex `model/list` returned ten visible models,
all offering Fast and one offering Ultrafast. Other accounts can receive a
different list, so the runtime reads each model's advertised service tiers.
[Alibaba Beijing](https://help.aliyun.com/zh/model-studio/token-plan-personal-overview) · [Alibaba Singapore](https://www.alibabacloud.com/help/en/model-studio/token-plan-personal-overview) · [Google-managed Claude](https://platform.claude.com/docs/en/build-with-claude/claude-models-on-google-cloud/google-managed) · [Claude Fast mode](https://platform.claude.com/docs/en/build-with-claude/fast-mode) · [Codex app server](https://github.com/openai/codex/tree/main/codex-rs/app-server)
