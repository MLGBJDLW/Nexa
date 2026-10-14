# Nexa documentation

Start with the [project README](../README.md) for installation and first use,
or [Contributing](../CONTRIBUTING.md) for local setup and verification.
English is canonical for maintained technical documentation; the
[Chinese README](../README.zh-CN.md) is the Chinese product entry point.

## Browse by purpose

| Category | What belongs here |
| --- | --- |
| [Use and configure Nexa](guides/README.md) | Models, knowledge, chat, Computer Use, scheduled tasks, voice, phone access, and local storage |
| [Runtime and architecture](runtime/README.md) | Subsystem ownership, streaming, orchestration, terminal and extension boundaries |
| [Extend Nexa](extensions/README.md) | Capability packages, MCP, skills, workflows, and explicitly labelled proposed extension surfaces |
| [Development and validation](development/README.md) | UX acceptance, localization, and reproducible agent evaluation |
| [Product direction](product/README.md) | Audience, product boundaries, and roadmap |
| [Tool reference](reference/README.md) | Tool behavior, scope, result contracts, and generated schema links |

## Common starting points

| Task | Start here |
| --- | --- |
| Connect a model or subscription | [Models and providers](guides/PROVIDERS_AND_MODELS.md), [subscription agents](guides/SUBSCRIPTION_AGENTS.md) |
| Add files and retrieve evidence | [Knowledge and retrieval](guides/KNOWLEDGE_AND_RETRIEVAL.md), [document input](guides/NATIVE_DOCUMENT_INPUT.md) |
| Understand desktop automation | [Computer Use](guides/COMPUTER_USE.md) |
| Configure recurring work | [Scheduled tasks](guides/SCHEDULED_TASKS.md) |
| Diagnose a long-running turn | [Streaming protocol](runtime/AGENT_STREAMING_PROTOCOL.md), [orchestration](runtime/ORCHESTRATION_RUNTIME.md) |
| Change runtime behavior | [Architecture](runtime/ARCHITECTURE.md), [Contributing](../CONTRIBUTING.md) |
| Validate or release a change | [Development guides](development/README.md), [contribution checks](../CONTRIBUTING.md) |

## Maintain the documentation

Place stable guides in the category matching their primary purpose and link
them from that category's README. Keep one authoritative document for each
contract; link related material instead of copying it. Every guide links back
to its category and this index. The root of `docs/` contains only this entry
point; add a category only when an existing one cannot hold the material.

Update the relevant guide when a command, schema, permission, workflow, or
platform requirement changes. Link implementation and tests for verification.
Label proposed behavior explicitly. Current repository documents describe the
current source; a release tag's documents and assets describe that release.

Run `npm run docs:check` to validate local links, headings, generated schema
links, and reachability through category indexes. After moving a guide, update
its incoming links, relative source links, and the category index together.

Dated investigations, source dumps, implementation research, and temporary
plans belong in Issues/PRs or the ignored `docs/local/` and `docs/research/`
work areas. The legacy `docs/architecture/` scratch tree remains ignored;
maintained architecture now lives in `docs/runtime/`.

Local agent instruction files and temporary patch payloads remain ignored and
must not be committed. `npm run repository:check` enforces this. Product prompts
and bundled skills remain maintained source assets. Release history and legal
notices keep their existing locations: [changelog](../CHANGELOG.md) and
[third-party notices](../apps/desktop/THIRD_PARTY_NOTICES.md).
