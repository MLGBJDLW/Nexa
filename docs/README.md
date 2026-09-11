# Nexa documentation

Start with the [project README](../README.md) for installation and first use,
or [Contributing](../CONTRIBUTING.md) for local development and verification.
These documents describe the current repository; a release tag's documents and
assets describe that release.

English is the canonical language for maintained technical documentation.
[README.zh-CN.md](../README.zh-CN.md) is the Chinese product entry point.

## Use Nexa

| Guide | Read it when you need to... |
| --- | --- |
| [Knowledge and retrieval](KNOWLEDGE_AND_RETRIEVAL.md) | Add sources, retrieve evidence, use collections, or interpret the knowledge graph |
| [Models and providers](PROVIDERS_AND_MODELS.md) | Configure API/local connections, select models, or understand capability and credential boundaries |
| [Subscription agents](SUBSCRIPTION_AGENTS.md) | Sign in to supported Copilot/Codex runtimes or understand their execution limits |
| [Phone access](remote-access.md) | Pair a phone, select LAN/public routes, use remote chat, or diagnose a connection |
| [Voice and Live](LIVE.md) | Use dictation, live audio/video input, records, and summaries |
| [Local HTML preview](local-html-preview.md) | Open interactive local HTML with an explicit asset list |
| [Scheduled tasks](SCHEDULED_TASKS.md) | Configure recurrence, approvals, unattended tools, or isolated repository work |
| [Office add-in](../integrations/office-addin/README.md) | Pair or deploy the separate Word/Excel/PowerPoint live adapter |
| [Tool reference](TOOLS.md) | Choose an agent tool and inspect its schema, scope, and result contract |

## Understand the system

[ARCHITECTURE.md](ARCHITECTURE.md) is the canonical architecture entry point.
[CONTEXT.md](../CONTEXT.md) defines the Agent Run and provider vocabulary.

| Contract | Authority covered |
| --- | --- |
| [Agent streaming protocol](AGENT_STREAMING_PROTOCOL.md) | Ordered Run Events, the outbox, durable completion, replay, and recovery |
| [Orchestration runtime](ORCHESTRATION_RUNTIME.md) | MoA, Nexus, budgets, prompt caching, delegation, verification, and evaluation |
| [Live file-tool streaming](LIVE_FILE_TOOL_STREAMING.md) | Partial previews, complete-argument execution, and resumable text writes |
| [Terminal and agent bridge](TERMINAL_AGENT_BRIDGE.md) | User-owned PTYs, conversation binding, approvals, and stop semantics |
| [Ecosystem architecture](ECOSYSTEM_ARCHITECTURE.md) | Extension ownership, user files, trust boundaries, and maturity |

## Extend Nexa

| Surface | Status and reference |
| --- | --- |
| Capability packages | [Manifest validation and discovery](CAPABILITY_PACKAGES.md); discovery does not install executable code |
| MCP connectors | [Supported connector lifecycle](MCP_CONNECTORS.md) and versioned file declarations |
| Skill packages | [Import, resources, dependencies, and reload](SKILL_PACKAGES.md) through existing tools |
| Workflow packages | [Built-in catalog and proposed portable format](WORKFLOW_PACKAGES.md) |
| Protocol exits | [Candidate/design metadata](PROTOCOL_EXITS.md); not a shipped standalone server |
| Native plugins | [Future runtime requirements](NATIVE_PLUGIN_RUNTIME.md); not a general third-party code loader |

## Product and contribution standards

- [Product direction](PRODUCT_DIRECTION.md): audience, pillars, and product boundaries.
- [Roadmap](ROADMAP.md): shipped foundations and ongoing priorities.
- [UX quality bar](UX_QUALITY_BAR.md): interaction, presentation, and acceptance.
- [Internationalization](I18N_GUIDELINES.md): namespace JSON, generated files, and locale review.
- [Contributing](../CONTRIBUTING.md): environment setup, checks, and PR expectations.
- [Changelog](../CHANGELOG.md): generated release history.
- [Third-party notices](../apps/desktop/THIRD_PARTY_NOTICES.md): bundled attributions.

## Documentation lifecycle

Stable guides and contracts belong directly under `docs/` and are linked from
this index. Normative runtime details are also reachable from
[Architecture](ARCHITECTURE.md). Keep one owner for each contract and link
related material instead of copying whole sections.

Update the relevant guide when a command, schema, permission, user workflow,
or platform requirement changes. Link implementation and tests for verification.
A design example must be labelled as proposed when the runtime does not parse
or execute it. A recent edit date alone does not establish correctness.

Run `npm run docs:check` from the repository root to validate maintained local
links and index coverage. Inspect technical claims against the implementation
and check external references separately.

Dated investigations, source dumps, implementation research, and temporary
plans belong in Issues/PRs or the ignored `docs/local/` and `docs/research/`
work areas. The former `docs/architecture/` tree and research-style Markdown
filenames remain ignored. Generated release history, legal notices, and
domain-specific bundled skill assets have their own purpose and are not
rewritten as current architecture.
