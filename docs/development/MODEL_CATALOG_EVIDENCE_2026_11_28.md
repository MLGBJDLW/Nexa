# Model catalog evidence: November 28, 2026

[Development documentation](README.md) · [Audit reference](MODEL_CATALOG_AUDIT.md)

Compared draft #604 at `4a450f62d252b715d26e6f214c59e58b4f321140`
with current master `3465dc74d2c4c423c03bfd9619fe6244f6e8f9da`.
Anonymous catalog and endpoint metadata only; no synthesis, image
generation, authenticated inference, or provider account checks.

| Exact scoped route | Evidence and action |
| --- | --- |
| OpenRouter MAI-Image [2.5](https://openrouter.ai/api/v1/images/models/microsoft/mai-image-2.5/endpoints) and [2.5 Pro](https://openrouter.ai/api/v1/images/models/microsoft/mai-image-2.5-pro/endpoints) | November 27 was the published retirement date. Both exact endpoints now return HTTP 404 with their own model ID; neither is in the successful anonymous image catalog. Keep exact removal tombstones and no replacement. MAI 2.7 and 2.7 Flash still have separate Azure endpoint listings. |
| OpenRouter [Recraft V4.1 Flash V2 free](https://openrouter.ai/api/v1/images/models/recraft/recraft-v4.1-flash-v2:free/endpoints) | November 28 was the published date and raw literal-colon GET returns HTTP 404 explicitly naming the free ID. The parsed image inventory contains the paid sibling, not the free ID. The [paid exact endpoint](https://openrouter.ai/api/v1/images/models/recraft/recraft-v4.1-flash-v2/endpoints) returned HTTP 200 with $0.007 per image advertised. The paid route remains separate, with no default or replacement change. |
| OpenRouter [Ling 3.2 Flash](https://openrouter.ai/api/v1/models/inclusionai/ling-3.2-flash/endpoints) | Exact paid Ant Digital listing advertises text/image input, text output, 1,048,576 context, 262,144 output, tools, and structured output. Add only as a non-default choice. The separate :free ID's exact endpoint returned 200 with empty endpoints and no dated retirement located, as did Qwen3 235B A22B Thinking 2507; report no current backing, not permanent removal. No inferred paid replacement. |
| [OpenRouter Nemotron 3.5 Content Safety](https://openrouter.ai/nvidia/nemotron-3.5-content-safety) | Paid-only November 30 notice is already in the draft baseline; DeepInfra still listed on the exact paid endpoint. Exact free sibling returns 200 with no endpoints, no confirmed permanent retirement/date. No preset change for either variant today. |
| [DeepInfra Hy4](https://deepinfra.com/tencent/Hy4) | Parent audit independently confirmed official public model metadata lists exact, case-sensitive `tencent/Hy4-preview` with `replaced_by=tencent/Hy4` and `deprecated=1795740866` (November 27 00:54:26 UTC, distinct from indexed notice's displayed 00:49 UTC). Exact old ID was omitted from the OpenAI-compatible list; no live redirect behavior tested and no built-in DeepInfra preset to modify. Neither lowercase aliases nor other providers inherit this conclusion. |
| Google direct 2.5 aliases and OpenRouter December dates | Existing exact direct Developer API reroute advisories and OpenRouter retirement dates retained separately. No paid, authenticated or after-cutoff check and no saved model changes. |

The Settings picker keeps retired rows from the bundled or capability
catalog even when a fresh image inventory omits them or stale data
advertises them. The exact saved ID is visibly retired and disabled; it
is not migrated. The OpenRouter request serializer rejects retired
catalog descriptors rather than generating a request for either the
retired route or an inferred successor.

## Coverage

The public-only dynamic recheck enumerated 96 existing preset endpoints
and 1,563 model projections after the optional Ling row, zero static
errors. Six appeared on implemented live probes: OpenRouter text 476,
image 68, embeddings 40, Decisions 44, video 30; Jina embeddings 30.
The remaining endpoint reasons were 22 needing credentials, 62 with no
implemented operation inventory and six unsupported. The sidecar probes
verified 39 speech models and 30 transcription models separately without
implying any supported Nexa runtime or account availability. ACP registry:
46 advertised, all 44 monitored IDs matched, 31 version differences.
No installs, subscriptions, custom or local endpoints altered. Account
and region-specific availability remain unknown unless separately stated.
