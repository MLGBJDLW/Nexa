# Contributing to Nexa

Start with the [documentation index](docs/README.md) and
[architecture](docs/ARCHITECTURE.md). Changes should preserve source scope,
provider identity, durable task state, and user control at the runtime boundary
that owns the behavior.

## Prerequisites

- Git and **Node.js 24**, matching the repository's CI runtime. Install both
  lockfiles with `npm ci`; the root and desktop are separate npm packages.
- Rust stable selected by [rust-toolchain.toml](rust-toolchain.toml). The workspace
  minimum Rust version is declared in [Cargo.toml](Cargo.toml).
- Platform dependencies from the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/):
  Windows needs Microsoft C++ Build Tools with Desktop development with C++, an
  MSVC Rust toolchain, and WebView2; macOS needs Xcode or its command-line tools;
  Linux needs the distribution's WebKitGTK 4.1 and associated development libraries.
- Optional feature runtimes depend on the work: FFmpeg/ffprobe and speech models
  for media ingestion, model files for OCR/local embeddings, and Office runtimes
  for native document acceptance.

For the exact Linux CI package set, see [ci.yml](.github/workflows/ci.yml).
The application bundle's macOS minimum is declared in
[tauri.conf.json](apps/desktop/src-tauri/tauri.conf.json).

## Development

From the repository root:

```bash
npm ci
npm ci --prefix apps/desktop
```

Then choose one command from `apps/desktop`:

| Command | Result |
| --- | --- |
| `npm run tauri -- dev` | Starts the native desktop app and Vite; the wrapper selects an available development port |
| `npm run dev` | Starts only the web frontend; native operations require the Tauri host or test mocks |
| `npm run build` | Checks i18n and TypeScript, then builds the desktop and phone frontend assets |
| `npm run tauri -- build` | Builds the native application and the platform's configured bundle |

Build resources are a separate requirement for a release-equivalent Office
bundle. Source resource directories contain placeholders. The
[release workflow](.github/workflows/release.yml) prepares:

- The reviewed PptxGenJS/Node closure with root `npm run office:pptxgenjs:bundle`.
- The pinned .NET Open XML validator as a self-contained binary for the target
  runtime identifier under `apps/desktop/src-tauri/resources/openxml-validator`.
- The reviewed files from `integrations/office-addin` under the corresponding
  Tauri resource directory.

Follow those platform-specific preparation steps before claiming a local
installer has the same Office resources as an official release. See the
[Office add-in guide](integrations/office-addin/README.md) for pairing and
trusted deployment; a normal desktop build does not provision Office trust.

## Verification

Run checks that exercise the changed boundary. A documentation-only change can
be verified without starting models, a desktop session, or native Office.

From the repository root:

```bash
npm run docs:check
git diff --check
```

For frontend changes, from `apps/desktop`:

```bash
npm test
npm run typecheck
npm run build
```

For browser behavior, install the test browser once and run the affected specs
from `apps/desktop`:

```bash
npx playwright install chromium
npm run e2e -- e2e/chat-terminal-dock.spec.ts
```

On Linux, Playwright may also need its system dependencies. On Windows, an
installed Edge can be selected in PowerShell:

```powershell
$env:NEXA_PLAYWRIGHT_CHANNEL = 'msedge'
npm run e2e -- e2e/remote-phone.spec.ts e2e/remote-setup.spec.ts
```

Playwright starts the frontend on port 4173. These tests use mocked desktop
boundaries where appropriate; they do not prove native device or provider
acceptance. The critical CI spec sets are listed in
[ci.yml](.github/workflows/ci.yml).

For Rust changes, from the repository root:

```bash
cargo fmt --all -- --check
cargo clippy -p nexa-core -- -D warnings
cargo test -p nexa-core
cargo clippy -p nexa-remote --all-targets -- -D warnings
cargo test -p nexa-remote
cargo check -p nexa-desktop
```

Useful focused checks also include root `npm run catalog:audit`,
`npm run catalog:audit:test`, `npm run release:workflow:test`, and
`npm run office:addin:test`. Inspect [package.json](package.json) and
[the desktop scripts](apps/desktop/package.json) for the current command set.

Keep verification claims specific: compile checks, browser mocks, live provider
authentication, real microphone/camera behavior, public tunnel reachability,
and native Word/Excel/PowerPoint acceptance are different evidence. Ignored
tests that use an account, public tunnel, or native application require an
intentional environment setup and are not ordinary CI coverage.

## Cargo feature boundaries

| Feature | `nexa-core` default | `nexa-desktop` default | Purpose |
| --- | --- | --- | --- |
| `local-embeddings` | Yes | Yes | ONNX/tokenizer-backed local embeddings |
| `ocr` | Yes | Yes | ONNX-backed OCR |
| `video` | No | Yes | Media analysis and local speech dependencies; runtime assets are still required |
| `onnx-runtime` | Through OCR/embeddings | Through core | Shared optional ONNX dependencies |
| `custom-protocol` | Not applicable | Yes | Tauri application asset protocol |

The manifests are authoritative:
[core](crates/core/Cargo.toml), [desktop](apps/desktop/src-tauri/Cargo.toml).
When changing feature boundaries, check the standalone combinations used in CI:

```bash
cargo check -p nexa-core --no-default-features
cargo check -p nexa-core --no-default-features --features local-embeddings
cargo check -p nexa-core --no-default-features --features ocr
cargo check -p nexa-core --no-default-features --features video
```

## Documentation and translations

- Use English for public technical documentation. Keep
  [README.zh-CN.md](README.zh-CN.md) aligned with the English README's
  capabilities, setup, and limitations.
- Update the smallest relevant contract when behavior changes. Distinguish
  implemented behavior, deployment requirements, and proposed formats.
- Link maintained documents from [docs/README.md](docs/README.md). Link code,
  schemas, manifests, and focused tests instead of copying volatile inventories.
- Keep dated investigations, source dumps, and temporary plans in Issues/PRs or
  the ignored `docs/local/` and `docs/research/` work areas.
- UI translations use namespace JSON files and generated TypeScript. Follow
  [I18N_GUIDELINES.md](docs/I18N_GUIDELINES.md), including all shipped locales.
- Run `npm run docs:check` after changing links, headings, or the documentation
  index. This checks local references and navigation, not external service uptime
  or the truth of a technical claim.

## Pull requests

Start from an up-to-date `master` and use a descriptive `feature/` or `fix/`
branch. Preserve unrelated local changes. Keep commits separated by behavior
and use Conventional Commit subjects, for example
`docs(remote): explain pairing and automatic routes` or
`fix(streaming): preserve terminal event ordering`.

Describe the user-visible result, the relevant runtime boundary, and the checks
performed. Review feedback and CI must refer to the final pushed commit. A
passing build does not replace review or native acceptance. Release notes and
versions are managed by the release workflow; do not rewrite historical
[CHANGELOG.md](CHANGELOG.md) entries for a documentation refresh.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Root `npm run build` is missing | Run frontend commands from `apps/desktop` |
| `link.exe` is missing | Install/repair the Windows C++ workload and MSVC toolchain; a GNU core check does not prove the native desktop build |
| GTK/WebKit pkg-config failure | Install the platform development packages from the Tauri prerequisites and compare with the Linux CI job |
| i18n generated files are stale | Edit namespace JSON, then run `npm run i18n:generate` and `npm run i18n:check` from `apps/desktop` |
| Playwright cannot find Chromium | Install its browser, or select installed Edge on Windows |
| Model, OCR, or Office runtime is unavailable | Check that feature's settings and downloaded/bundled assets; record the missing runtime separately from a code failure |

For bug reports, include the Nexa version, platform, relevant connection type,
reproduction steps, expected/actual behavior, and redacted logs. Keep keys,
pairing codes, private source contents, and access tokens out of public reports.
