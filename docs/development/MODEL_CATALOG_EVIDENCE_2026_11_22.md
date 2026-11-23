# Model catalog evidence: November 22, 2026

[Development documentation](README.md) · [November 21 evidence and endpoint matrix](MODEL_CATALOG_EVIDENCE_2026_11_21.md)

Audited master `60b3169` and existing draft [#589](https://github.com/MLGBJDLW/Nexa/pull/589)
at `8e0a715`; this update is incremental on that draft. No credentialed
inventories, inference, account/installation state, default changes, index
changes, auto-merges or releases were performed.

## Catalog changes

- **OpenRouter Inkling 2 Mini:** Unlike the November 21 review snapshot,
  `thinkingmachines/inkling-2-mini` now appears in the text list and has
  two active backing entries. List/Prime Intellect context 1048576, Io Net
  context 262144, both output maximum 65536. Inputs text/image, output
  text, tools and structured output. Optional reasoning advertises
  none/minimal/low/medium/high/max and default high. Do not treat the
  advertised top-provider context as a guarantee across backends, or infer
  additional hosted modes from released weights. Opt-in only.
  [Model card](https://openrouter.ai/thinkingmachines/inkling-2-mini/)
  · [Exact endpoints](https://openrouter.ai/api/v1/models/thinkingmachines/inkling-2-mini/endpoints).
- **OpenRouter DaisyRose and MiniMax:** The [live models list](https://openrouter.ai/api/v1/models)
  now specifies DaisyRose optional reasoning, default enabled at xhigh,
  with none/low/medium/xhigh; add only those controls. MiniMax M2.7 now has an
  active Groq backing entry; its exact endpoint and the public models list
  agree on output max 131072 and no structured-output parameters. Keep
  tools and reasoning; do not generalize to other MiniMax routes.
  [M2.7 endpoints](https://openrouter.ai/api/v1/models/minimax/minimax-m2.7/endpoints).
- **Native xAI Grok Build 0.1:** The November 19 [release note](https://docs.x.ai/developers/release-notes#grok-build-01-end-of-life)
  explicitly says the old slug stopped serving. Retain its ID as a tombstone
  and remove the stale `grok-code-fast-1` migration link into it. The
  [standalone model card](https://docs.x.ai/developers/models/grok-build-0.1)
  still exists and displays specs; it is not an authenticated availability
  check or a resumption notice. The distinct OpenRouter slug has
  [no currently listed backing routes](https://openrouter.ai/api/v1/models/x-ai/grok-build-0.1/endpoints)
  but no independent retirement announcement was verified, so it is unchanged.
- **OpenRouter Liquid D1:** The published November 20 withdrawal is now past,
  the Decisions list lacks the old ID, and the [exact D1 endpoint response](https://openrouter.ai/api/v1/models/liquid/d1/endpoints)
  returned HTTP 200 with no backing routes. Preserve D1 as a tombstone.
  [D1.1](https://openrouter.ai/liquid/d1.1) still has a Liquid backing
  and a November 24 cutoff, so remains deprecated, not removed. Advise both
  to the currently advertised stable [D1.2](https://openrouter.ai/liquid/d1.2);
  the provider's [rolling alias](https://openrouter.ai/~liquid/d1-latest/)
  currently resolves to it, but no alias or saved selection is changed. This
  is Nexa's suggestion; the existing resolver requires manual selection for
  removed Decisions models. D1 Ultra's explicit current expiration is null,
  so no change to it or its rolling alias.

## Observed but not changed

- The same draft already covers the future Beijing Ark November 24 14:00
  UTC+8 cutoff and the distinct Vertex lifecycle. Ark Seed 2.0 Pro's
  provider-side replacement remains pending; no automatic migration was
  implemented. Other official lifecycle checks did not establish a new
  in-scope change: [OpenAI](https://developers.openai.com/api/docs/deprecations),
  [Anthropic](https://platform.claude.com/docs/en/about-claude/model-deprecations),
  [Bedrock](https://docs.aws.amazon.com/bedrock/latest/userguide/model-lifecycle.html),
  [Azure](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/model-retirement-schedule),
  [Mistral](https://docs.mistral.ai/inference/model-lifecycle).
- [Alibaba Virginia Qwen4 Max](https://www.alibabacloud.com/help/en/model-studio/qwen4-max)
  uses a Virginia workspace URL and scoped API key; the Nexa text preset
  matcher does not currently recognize that region as a built-in identity.
  Do not widen the Singapore row or reuse its key. [Together Qwen4 Max](https://www.together.ai/models/qwen4-max)
  is a separate hosting route with ID `Qwen/Qwen4-Max` and mandatory streaming;
  no native Together preset exists. Provider-specific custom integration was
  not validated. Neither changes a default, installed model or another route.
- [Recraft V4.1 Flash V2 free](https://openrouter.ai/recraft/recraft-v4.1-flash-v2%3Afree/)
  has a November 28 sunset announcement but its exact free ID is not in
  the dedicated public inventory or bundled preset. The paid SKU is distinct
  and unchanged. [OpenAI GPT Live 1.1](https://developers.openai.com/api/docs/changelog)
  uses Live Sessions, not an existing chat, TTS or STT adapter. Runway H3
  Image uses the dedicated image operation, not Nexa's video provider.
- The Runway [OpenAPI](https://docs.dev.runwayml.com/openapi.json) matched the
  draft contract snapshot and its advertised T2V/I2V/V2V IDs (29/30/17) were
  represented on the draft. This does not mean all operations are integrated;
  in particular, Seedance Flash has only the draft's prompt-only operation.
  The public video inventory omitted two Alibaba Wan routes and three Kling
  routes versus yesterday; no route-specific retirement announcements were
  verified. Five missing text IDs are likewise watch candidates, not removals.
- [Perplexity contextual embeddings](https://openrouter.ai/docs/api/api-reference/embeddings/submit-an-embedding-request)
  require exact query/document intent and grouping for contextual documents;
  the flat existing adapter cannot preserve that semantics. No new preset,
  paid route or embedding index migration.
- The [public OpenRouter ZDR snapshot](https://openrouter.ai/api/v1/endpoints/zdr)
  contained 936 records when checked: published MiniMax M2.7 Groq and M2.5
  Venice/DigitalOcean endpoint tags all appeared in the ZDR list with
  `safety_retention:false` and status 0. This is a current route-metadata
  observation, not assurance for all providers, unconstrained requests, past
  requests or any account. No policy, default, or route change was made.

## Verification scope

Seven operation manifests yielded 94 endpoints. The final audit reported
1532 static model projections and zero static errors. All six public reads
succeeded; 22 unprobed endpoints require credentials, 60 do not have an
operation listing implemented by the auditor, and six use unsupported
or runtime discovery. Endpoint identities are unchanged from the
[November 21 matrix](MODEL_CATALOG_EVIDENCE_2026_11_21.md#coverage-and-checks).
No live proof of region/account-specific model access is implied.

| Operation | Static endpoints | Public inventory actually read |
| --- | ---: | --- |
| Text | 27 | OpenRouter 476 |
| Image | 9 | OpenRouter 70 |
| Embedding | 17 | OpenRouter 40; Jina 30 (16 embedding/CLIP IDs checked against preset) |
| Speech to text | 17 | No public operation listing in the auditor |
| Text to speech | 12 | No public operation listing in the auditor |
| Decisions | 7 | OpenRouter 44 |
| Video | 5 | OpenRouter 30; separate Runway public OpenAPI spot check |

The two subscription presets (GitHub Copilot and ChatGPT/Codex) still
discover at sign-in; local Ollama, LM Studio, Whisper/Sherpa and custom
providers need actual installation/connection. For external ACP, the official
[registry](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json)
listed 46 entries in public indexed data; all 44 Nexa registry pins matched
an ID but 21 versions differed. Distribution, digest and installation were
not verified; pins remain unchanged. No automatic refresh or installation.

All 44 targeted provider contracts, catalog audit fixtures, five Registry
and Runway JS tests, documentation checks and diff checks passed. No local
Rust toolchain; exact-head GitHub CI is reported separately.
