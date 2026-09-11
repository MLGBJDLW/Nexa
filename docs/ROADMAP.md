# Product roadmap

Nexa's direction is a local-first desktop assistant for everyday knowledge,
document, and desktop work. This roadmap separates existing foundations from
ongoing product priorities; it is not a release schedule or a promise that every
provider, operating system, or device supports every feature.

See [GitHub Releases](https://github.com/MLGBJDLW/Nexa/releases) and
[CHANGELOG.md](../CHANGELOG.md) for shipped release history.

## Shipped foundations

- Local ingestion, hybrid retrieval, source filters, Recall Mode, collections,
  and project context.
- Knowledge graph navigation with evidence links, source/path/type filters, and
  bounded graph queries.
- Durable Agent Runs, ordered events, checkpoints, streamed output,
  reconciliation, archive/restore, and per-turn history.
- Shared model catalogs, endpoint-aware capability resolution, model/reasoning
  selection, supported subscription runtimes, and configured API workers.
- Scoped file tools, Office artifact validation/review, local file previews,
  Browser Workspace, and a separately paired Office.js live adapter.
- A conversation-linked terminal and structured browser/computer tools with
  explicit observation, permission, and lifecycle boundaries.
- Editable-composer dictation, Live observation/summary sessions, QR-paired
  phone chat, encrypted LAN access, and optional managed/fixed public routes.
- Workflow templates, durable scheduled occurrences, unattended tool policies,
  and isolated patch execution under supported conditions.
- Ten UI locales, theme resources, and user-owned skill/connector declarations.

These are implementation surfaces. Their operating limits are documented in
the [focused guides](README.md), including platform requirements, account
capabilities, runtime assets, and native acceptance boundaries.

## Current priorities

### Reliability and trust

- Preserve one durable outcome across streaming, retry, pause, restart, and
  remote reconnection.
- Keep source scope, device authorization, provider identity, and user approvals
  enforced by the owning runtime.
- Improve provider/device coverage with real acceptance evidence while retaining
  fast deterministic contract tests.
- Keep resource use, background work, graph queries, and live queues bounded.

### Everyday usability

- Strengthen Search, Collections, Projects, and Chat as a continuous working set.
- Improve onboarding, document review, actionable failures, and task recovery.
- Keep advanced agent controls understandable and maintain keyboard access,
  reduced motion, phone layouts, and all shipped locales.
- Make model setup and capability limitations clear without hard-coding
  short-lived model-version claims into product docs.

### Extension maturity

- Preserve the supported MCP, skill, theme, and built-in workflow paths.
- Evolve capability and workflow package formats only with parser, trust,
  lifecycle, and executable host support.
- Keep MCP export at candidate status and ACP/A2A at design status until an
  actual server/executor is delivered and tested.
- Open a native plugin runtime only after the safer extension surfaces and
  isolation requirements are satisfied.

## Change discipline

Move a priority into shipped foundations only when its behavior, restrictions,
and verification can be linked. Update the smallest owning guide as part of the
same change. Keep implementation checklists and dated investigations in their
Issue/PR or ignored local research area rather than adding competing roadmaps.
