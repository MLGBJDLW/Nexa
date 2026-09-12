# Models and providers

Nexa separates a saved connection, a selected model, and the capabilities of the
exact endpoint behind that connection. Model names alone do not authorize a
protocol, context limit, reasoning control, or credential route.

## Choose a connection

| Connection | Setup and execution |
| --- | --- |
| API provider | Add a provider in Settings, configure its base URL and credentials, save it, then select an available model in Chat |
| Local model service | Configure the supported local endpoint and run that service on the computer; local inference still requires the service and model files |
| Subscription agent | Add GitHub Copilot or ChatGPT / Codex, sign in through its official runtime, and save the provider; see [Subscription agents](SUBSCRIPTION_AGENTS.md) |

Use the model picker and refresh/discovery actions to inspect current choices.
If a custom endpoint does not support discovery, use its documented model ID.
A saved model choice is not proof that the endpoint will accept a request.
Verify a simple conversation before depending on that connection for a workflow.

API keys and subscription logins are separate credential paths. A subscription
does not become an API key, and selecting an API-only feature does not silently
switch to another connection. Model availability, entitlement, cost, and service
limits remain properties of the configured account and endpoint.

## Capability resolution

The shared [provider presets](../shared/provider-presets.json),
[core provider catalog](../crates/core/src/provider_catalog.rs), and
[capability registry](../crates/core/src/capability_registry) define known
endpoint/model facts. Runtime discovery and explicit configuration supplement
that catalog. The frontend projects the resolved choices instead of maintaining
an independent model inventory.

- Chat, tool use, images, audio, video, embeddings, and speech are distinct
  capabilities. Selecting a model for one does not enable all the others.
- Reasoning effort and thinking budgets appear only where the selected route
  supports them. Nexa orchestration profiles such as Code Ultra are not provider
  reasoning values.
- Unknown custom endpoints remain provider-managed where their capacity or
  protocol cannot be established. Nexa must not guess a public-provider limit
  from an alias or recognizable model name.
- Credentials stay scoped to their configured provider/endpoint. Model discovery
  is not permission to forward credentials to another host.
- Image and speech connections keep their own configuration and capabilities;
  changing one provider must not copy another provider's keys or model settings.

For output continuation, accepted-route replay, context management, and explicit
worker budgets, see [Orchestration runtime](ORCHESTRATION_RUNTIME.md).

## Model retirement

Confirmed retirements are endpoint-scoped tombstones in the shared catalog.
They override old discovery caches and are excluded from desktop and phone
choices. The API provider boundary rejects both streaming and non-streaming
requests to those IDs, including documented retired aliases. Saved connections
remain editable with their original model ID and a suggested replacement;
Nexa does not select a differently priced model without a user selection.

`deprecated` and `legacy` do not mean unavailable. An announced future shutdown
does not remove a model early, and a missing account discovery result alone
does not establish retirement. A redirected retired ID also needs an explicit
replacement choice: its old model no longer serves requests even if the host
still accepts that spelling. Local and private deployments retain their own
availability authority.

For example, the [Moonshot model list](https://platform.kimi.ai/docs/models)
retires K2.5 and Moonshot V1, while [Alibaba's retirement notice](https://help.aliyun.com/zh/marketplace/three-party-direct-supply-model-kimi-k2-5-offline-notification)
applies to its third-party direct-supply route. Alibaba's separately hosted
`kimi-k2.5` is not the same route as `kimi/kimi-k2.5`. Similarly,
[DeepSeek's current service notice](https://api-docs.deepseek.com/quick_start/pricing/)
keeps V4 Pro available after the previously announced deadline. Record current
source links and verification dates when changing lifecycle facts.

## Select the right execution surface

| Task | Boundary |
| --- | --- |
| Normal API chat | Nexa owns the agent loop and dispatches available tools under current policies |
| Subscription chat | The official runtime owns its model loop; Nexa owns tool dispatch and durable conversation state |
| Subagent work | Configured API workers selected through `list_subagent_models` and `agent_config_id`; subscription child runtimes are not implemented |
| Dictation | A speech connection that actually supports the selected input and interim/final delivery mode |
| Live | A supported native realtime protocol or a configured vision plus streaming-transcription route; see [Voice and Live](LIVE.md) |
| Embeddings | A configured local or API embedding runtime; API embeddings disclose the text sent for embedding |
| Phone chat | Desktop-backed model choices and execution; the phone does not receive model credentials |

Controls changed during an active response apply according to their owning
next-turn selection contract; they must not mutate an already accepted provider
route mid-sample. A failed or unavailable route should report its actual failure.

## Troubleshooting

| Symptom | Next check |
| --- | --- |
| Signed in, but no subscription provider in the picker | Complete Add Provider and save the provider after enrollment |
| Models cannot be refreshed | Check the endpoint, account entitlement, and discovery support; retain or enter a documented model ID instead of inventing one |
| Reasoning or a media mode is unavailable | Inspect capabilities for the exact selected connection; model-family support is insufficient |
| Dictation only appears after recording stops | Check the adapter's delivery mode; a batch engine is not an interim streaming adapter |
| Context or output limits differ between connections | Check endpoint/model capacity authority and explicit overrides before changing orchestration budgets |

## Implementation and checks

- [Model choices](../apps/desktop/src/features/models/modelChoices.ts) and
  [desktop model resolution](../apps/desktop/src-tauri/src/commands/model_choices.rs).
- [Subscription execution](../apps/desktop/src-tauri/src/subscription_runtime).
- [Catalog audit](../scripts/model-catalog-audit.mjs): run root
  `npm run catalog:audit` and `npm run catalog:audit:test` after catalog changes.
- [Provider settings browser coverage](../apps/desktop/e2e/settings-provider-models.spec.ts).

Static catalogs and mocked UI tests do not establish real account entitlement
or live provider availability. Keep those acceptance results separate.
