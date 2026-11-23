# Model catalog evidence: November 23, 2026

[Development documentation](README.md) · [November 21 endpoint matrix](MODEL_CATALOG_EVIDENCE_2026_11_21.md#coverage-and-checks) · [November 22 audit](MODEL_CATALOG_EVIDENCE_2026_11_22.md)

Reviewed the tree of [merged PR #589](https://github.com/MLGBJDLW/Nexa/pull/589)
(`4eba2073`). Public evidence does not establish account entitlement or a
successful generation. No paid requests, credentialed discovery, agent
installation, account changes, automatic model switches, index migration,
merge or release were performed by this audit.

## OpenRouter Images

| Exact ID | Announced cutoff | Change |
| --- | --- | --- |
| `recraft/recraft-v4.1-flash-v2:free` | November 28, 2026 | Record an exact, non-recommended deprecated row for the newly listed preset |
| `openai/gpt-image-1-mini` | December 1, 2026 | Mark the existing image row deprecated |
| `openai/gpt-5-image-mini` | December 1, 2026 | Already tracked on both surfaces; no change |
| `openai/gpt-5-image` | December 11, 2026 | Mark the existing image row deprecated; text already tracked |
| `openai/gpt-image-2` | February 10, 2027 | Mark the existing image row deprecated |

The [general image-filtered model list](https://openrouter.ai/api/v1/models?output_modalities=image)
gives an exact expiration for all five. The [dedicated image list](https://openrouter.ai/api/v1/images/models)
currently includes them but omits the expiration field. Field omission is not
a withdrawal. Dedicated endpoint reads corroborated present backing for the
three newly updated OpenAI models and Recraft's free route. The latter
advertises text-to-image, an aspect ratio selector and a batch size of 1–6;
the adapter uses its standard PNG fallback when output format is not declared.
No reference images, output quality selector, or pricing assumptions added.

[Recraft free notice](https://openrouter.ai/recraft/recraft-v4.1-flash-v2%3Afree/)
· [Image 1 Mini](https://openrouter.ai/openai/gpt-image-1-mini)
· [GPT-5 Image Mini](https://openrouter.ai/openai/gpt-5-image-mini)
· [GPT-5 Image](https://openrouter.ai/openai/gpt-5-image)
· [GPT Image 2](https://openrouter.ai/openai/gpt-image-2).

Do not mark future events removed, replace with a different model, map free
to paid or reuse the GPT-5 Image Mini card's image-only suggestion as an
automatic fallback: that target retires the same day. Native OpenAI already
carries its separate [deprecation notices](https://developers.openai.com/api/docs/deprecations)
and is unchanged. Existing configuration IDs stay exact. Dynamic discovery
already merges bundled lifecycle metadata into matching live IDs. Tests
exercise that behavior and confirm the generated request retains the selected
ID; no discovery or request runtime code was changed.

## Other findings, unchanged

- OpenRouter dedicated and generic video lists returned the same 30 IDs.
  Five bundled selectable Wan 2.6/2.7, Kling 3.0 Pro/Std and Kling
  Video O1 routes were absent; exact generic endpoint lookup returned no
  backing entries, dedicated exact lookup returned 404. This corroborates
  current public unavailability, not permanent retirement or a migration
  destination. The sixth absent historical ID, Seedance 1.5 Pro, is already
  retired in Nexa. No selections were changed. [Video catalog](https://openrouter.ai/api/v1/videos/models).
- [DeepInfra announced](https://status.deepinfra.com/models/embeddinggemma-300m)
  that its `google/embeddinggemma-300m` would redirect to `google/embeddinggemma-2`
  November 23 at 17:09 UTC, still a future event at this run. There is no
  dedicated Nexa DeepInfra embedding preset. The new model's documented
  [768-dimensional output](https://deepinfra.com/google/embeddinggemma-2/api)
  does not prove vector-space compatibility; no custom or local selection,
  index assumption or provider-side redirect testing was done. OpenRouter
  Perplexity contextual embedding still needs document/query request support.
- [Beijing Ark](https://docs.volcengine.com/docs/ark/model-deprecation-notice?lang=zh)
  four November 24 retirements remain future at 14:00 China Standard Time
  (06:00 UTC). Guidance is already on master from #589. Continue to distinguish
  the provider-side residual replacement from Nexa's manual advisory.
- [Anthropic's native Sonnet 4.5 notice](https://platform.claude.com/docs/en/about-claude/model-deprecations)
  covers both the dated `claude-sonnet-4-5-20250929` snapshot and the undated
  alias on November 30. The bundled alias already has that deadline; do not
  create a new selectable row merely to represent the dated snapshot or alter
  account-discovered IDs. This notice does not apply to other providers.
- The Runway [video OpenAPI](https://docs.dev.runwayml.com/openapi.json) still
  matched the previous checked snapshot, including the distinct Seedance 2.5
  Flash contract. Jina public list covered all 16 bundled embedding/CLIP IDs.
  OpenRouter Decisions audio drift remains contradictory to the exact model
  endpoints. No other text, speech recognition, speech synthesis,
  or operation-specific change met the implementation bar.

## Verification

The final public-only audit enumerated 94 operation endpoints and 1533 static
projections, with zero static errors. All six attempted public inventories
succeeded. Of the remaining 88, 22 require credentials, 60 lack implemented
operation inventory, and six use unsupported or runtime discovery. This is not
a full read of upstream inventories or region/account eligibility.

| Operation | Static endpoints | Inventory actually read |
| --- | ---: | --- |
| Text | 27 | OpenRouter 476 |
| Image | 9 | OpenRouter dedicated 71; general image-filtered 74 for cross-check |
| Embedding | 17 | OpenRouter 40; Jina 30 |
| Speech recognition | 17 | No public operation probe available in the auditor |
| Speech synthesis | 12 | No public operation probe available in the auditor |
| Decisions | 7 | OpenRouter 44 |
| Video | 5 | OpenRouter 30; separate Runway public OpenAPI |

The 46 configured external agents and two subscription runtime presets are
not included in those 94 endpoints. The indexed public [ACP registry](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json)
contained 46 live IDs and all 44 tied to Nexa's pins, with 21 version differences.
Version listings are not installation, integrity or account-model evidence. No
pin was changed. Copilot/Codex subscription, custom endpoints and installed
Ollama, LM Studio and local voice models remain runtime/credential-specific.

Six focused desktop image catalog cases, catalog fixtures, five registry/Runway
JS checks, docs and diff checks passed. A Rust synthetic discovery/request
regression is included for exact-head CI; Cargo was not available locally.
