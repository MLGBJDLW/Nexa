# Nexa

> A local-first desktop assistant for your files, knowledge, and everyday work.

[![CI](https://github.com/MLGBJDLW/Nexa/actions/workflows/ci.yml/badge.svg?event=pull_request)](https://github.com/MLGBJDLW/Nexa/actions/workflows/ci.yml?query=event%3Apull_request)
[![Release](https://github.com/MLGBJDLW/Nexa/actions/workflows/release.yml/badge.svg)](https://github.com/MLGBJDLW/Nexa/actions/workflows/release.yml)
[![Latest release](https://img.shields.io/github/v/release/MLGBJDLW/Nexa)](https://github.com/MLGBJDLW/Nexa/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[简体中文](README.zh-CN.md) · [Download](https://github.com/MLGBJDLW/Nexa/releases/latest) · [Documentation](docs/README.md) · [Contributing](CONTRIBUTING.md)

Nexa brings local search, evidence-based conversations, document work, and agent
tools into one desktop workspace. Add folders of notes, PDFs, spreadsheets,
presentations, or images; find relevant material, inspect citations, collect
evidence, and ask the assistant to help you finish a task.

The desktop owns your indexes, conversations, collections, and task history.
Cloud model and service connections receive the input needed for the requested
operation. Local models are also supported. Local-first does not mean every
feature is offline: model downloads, cloud inference, web search, connectors,
and optional public phone access use the network.

## What you can do

| Area | Current capabilities |
| --- | --- |
| Knowledge and search | Folder ingestion, incremental indexing, OCR, keyword and vector retrieval, source filters, Recall Mode, and a knowledge graph for navigating back to supporting documents |
| Conversations | Cited answers, collection and project context, durable task history, streaming tool activity, checkpoints, archive/restore, and Markdown with math and Mermaid diagrams |
| Models and agents | API and local model connections, endpoint-aware capability discovery, model/reasoning selection, supported subscription agents, reusable skills, and configured API subagents |
| Files and Office | Scoped file edits, document analysis and generation, Office artifact validation and review, local previews, and a separately paired Office.js add-in |
| Desktop and browser | A conversation-linked terminal, Browser Workspace, authorized HTML previews, structured browser tools, and Windows computer interaction with observation and approval boundaries |
| Voice and Live | Dictation into the editable composer, live microphone/camera/screen input where the selected connection supports it, and saved text observations and summaries |
| Phone access | QR pairing, existing conversations, model choices, dictation, file previews, individual approvals, and Live through encrypted LAN or configured public routes |
| Repeatable work | Collections, project memory, workflow templates, scheduled tasks, MCP connectors, and user-owned skill and theme files |

The graph is a navigation index, not evidence by itself. Open the underlying
documents before relying on a relationship. Tool availability depends on the
host, configured connection, source scope, and permissions; an API-compatible
endpoint does not automatically implement every provider feature.

## Install and start

Download an installer from [GitHub Releases](https://github.com/MLGBJDLW/Nexa/releases/latest).
Choose an asset for your operating system and CPU architecture from that release:

| Platform | Release package |
| --- | --- |
| Windows | NSIS installer (`.exe`) |
| macOS | Disk image (`.dmg`); the application configuration requires macOS 14 or later |
| Linux | AppImage |

An individual release's assets are the authority for available builds. Native
computer-control tools are Windows-specific; browser, microphone, media, and
Office integration support also depends on the host and installed runtimes.

1. Open Nexa and configure a model connection in Settings. See
   [Models and providers](docs/PROVIDERS_AND_MODELS.md) for API, local, and
   subscription connection boundaries.
2. Add a local folder in Sources and wait for indexing. Start with a small
   folder and check exclusions before adding a large collection.
3. Search for a known document, inspect the result, and ask a question with
   that source in scope. Save useful evidence into a Collection.
4. For document work, review the generated artifact and its validation result.
   For phone use, follow [Phone access](docs/remote-access.md).

Supported document inputs include Markdown, text, logs, PDF, DOCX, XLSX, PPTX,
and images. OCR and media processing require their corresponding models and
runtimes; enabling a Cargo feature alone does not install every runtime asset.

## Privacy and control

- Local files, indexes, conversations, and task records stay on the desktop by
  default. Optional API embeddings and cloud tools send their selected input to
  the configured service.
- Source exclusions and redaction rules help control what is indexed and sent
  to models. Review those settings before indexing sensitive folders.
- File, shell, browser, computer, terminal, and connector operations have their
  own enforced access and approval policies.
- A paired phone can read conversations and authorized previews, start agent
  work, and respond to individual approval requests. Revoke devices from the
  desktop when they should no longer have access.
- Public tunnel providers carry remote traffic. Use encrypted LAN access or
  your own fixed HTTPS route when that better fits your deployment.
- Nexa does not include a product telemetry pipeline. External services have
  their own data handling policies.

## Build from source

Use **Node.js 24**, Rust stable from [rust-toolchain.toml](rust-toolchain.toml),
and the platform dependencies listed in the [contribution guide](CONTRIBUTING.md).

```bash
git clone https://github.com/MLGBJDLW/Nexa.git
cd Nexa
npm ci
npm ci --prefix apps/desktop
cd apps/desktop
npm run tauri -- dev
```

For frontend work, run `npm run dev` from `apps/desktop`. This starts the web
frontend, not the native desktop runtime. For a desktop bundle, run
`npm run tauri -- build` from the same directory. See
[development and verification](CONTRIBUTING.md#development) for resource
preparation, feature flags, tests, and platform troubleshooting.

## Architecture and repository

Nexa uses Tauri 2, React 19.2, React Router 8.3, TypeScript, Rust, and SQLite.
Exact dependency versions live in the package manifests and lockfiles; provider/model availability lives in the
shared catalog and the configured endpoint's discovery results.

```text
apps/desktop/               React desktop and phone frontends
apps/desktop/src-tauri/     Native host, commands, browser, terminal, remote bridge
crates/core/               Agent runtime, retrieval, persistence, tools, media
crates/remote/             Pairing, HTTP/WebSocket transport, TLS, tunnels
shared/                    Provider and capability catalogs
integrations/office-addin/  Separately paired Office.js client
tools/                     Supporting validators and runtimes
scripts/                   Repository verification and release tooling
docs/                      Maintained user and engineering documentation
testdata/                  Test fixtures
```

Start with [Architecture](docs/ARCHITECTURE.md) for system ownership and
[the documentation index](docs/README.md) for the focused contracts. Product
priorities are maintained in [the roadmap](docs/ROADMAP.md); experimental and
proposed extension formats are explicitly identified in their references.

## Languages

The interface supports English, Simplified Chinese, Traditional Chinese,
Japanese, Korean, Spanish, French, German, Portuguese, and Russian.

English is the canonical language for public technical documentation. The
[Chinese README](README.zh-CN.md) covers the same core capabilities, setup,
and limitations. UI translation maintenance follows the
[internationalization guide](docs/I18N_GUIDELINES.md).

## Contributing and license

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a change. Report problems
through [GitHub Issues](https://github.com/MLGBJDLW/Nexa/issues), including the
release, platform, reproduction steps, and relevant redacted diagnostics.

Nexa is licensed under [MIT](LICENSE). Bundled third-party attributions are
listed in [Third-party notices](apps/desktop/THIRD_PARTY_NOTICES.md).
