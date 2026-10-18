# Generative chat UI

[Documentation](../README.md) / [Category overview](README.md)

Nexa can show structured, interactive views and self-contained HTML inside the
existing chat transcript. The same Markdown renderer is used by desktop and
phone chat, and saved message text reconstructs a view without calling a model
again. Plain text, Markdown tables, Mermaid and file previews remain available.

## Choose a surface

| Surface | Use | Runtime behavior |
| --- | --- | --- |
| `mermaid` fence | Static relationships, sequences and processes | Existing sanitized SVG renderer |
| `nexa-ui` fence | Metrics, sortable/filterable tables, signed bar charts, steps and tabs | Versioned data validated and rendered by native React components |
| `nexa-html` fence | Local calculators, simulations and bespoke HTML/CSS/JS explanations | Inert static preview streams; scripts run only after the user chooses Run |
| Ordinary `html` fence | Source code examples | Code first, optional Preview using the same HTML surface |
| Local HTML file | A deliverable with explicit supporting assets | Existing `open_in_nexa` and Browser Workspace contract |

The existing [visual-explanations skill](../../crates/core/assets/skills/visual-explanations/SKILL.md)
owns authoring guidance and templates. It selects a visual when that visual
improves the answer. It no longer requires every matching answer to contain
Mermaid. Skills guide output quality; validation and sandbox code enforce the
format and execution boundary.

## Data contract and streaming

The final assistant answer may end with a `nexa-followups` fenced JSON array of
two or three `{label, prompt}` objects, ranked by usefulness. Desktop chat shows
these as next-task buttons only after the response finishes. A click sends the
selected prompt through the normal composer. The first prompt also appears as
ghost text in an empty composer: Tab fills the draft, and sending remains a
separate action. Escape dismisses suggestions for that answer. Existing drafts,
attachments and active responses suppress the suggestions.

Only a complete top-level terminal fence in the latest assistant message can
activate suggestions. Quoted examples, commentary, invalid payloads and older
answers cannot create actions. Labels and prompts are plain bounded strings;
there is no script execution or additional recommendation-model request.

Structured views use `{version: 1, title, summary?, blocks}`. The complete
supported shapes are in the [format reference](../../crates/core/assets/skills/visual-explanations/references/structured-ui.md).
The [validator](../../crates/core/assets/skills/visual-explanations/resources/chat-ui.ts)
is imported by both the frontend and the bundled Node 24 validation helper, so
they accept the same format. The [example](../../crates/core/assets/skills/visual-explanations/resources/examples.json)
is also a regression fixture.

Unknown fields/components, invalid versions, mismatched table rows, non-finite
numbers and excessive nesting produce an inline diagnostic with the original
source. They do not execute an expression or break the rest of the answer.
During streaming, only fully closed, validated root blocks are displayed; final
malformed JSON is never silently repaired. Rendering capacity is bounded to
512 KiB, 128 blocks and four levels of nested tabs. Large datasets belong in
spreadsheet/file artifacts; the visual can show a useful subset.

Tables filter locally and sort numbers numerically. Tabs support arrow keys,
Home and End. Bar charts retain a common zero baseline and show the supplied
numbers and units. Data remains model/user supplied: presentation does not
validate factual claims. The surrounding answer should state conclusions,
sources and assumptions and remain useful without the interactive view.

## HTML execution boundary

Static previews use DOMPurify, a restrictive document CSP and an empty iframe
sandbox. Scripts, event handlers, navigation targets and embedded frames are
removed. HTML/CSS can therefore update while text streams without executing
partial scripts or allocating server resources per token.

After completion, **Run scripts** obtains a temporary preview from the shared
[HTML preview store](../../crates/remote/src/html_preview.rs). The desktop
[custom-protocol adapter](../../apps/desktop/src-tauri/src/chat_ui_preview.rs) and
remote HTTP adapter use this same store and security policy. The application
page's `script-src 'self'` remains unchanged.

Execution has two iframe layers:

1. The trusted outer document contains only a fixed iframe. Its CSP restricts
   child navigation to the preview origin and disallows its own scripts.
2. The generated inner document has an opaque origin (`allow-scripts`, without
   `allow-same-origin`) and a document CSP that permits inline scripts/styles
   and embedded data media. Network, workers, child frames, forms, base URLs,
   popups, downloads, clipboard, camera, microphone and host capabilities are
   unavailable. There is no tool, Tauri IPC, MCP Apps or OpenAI bridge.

The outer frame matters: `connect-src 'none'` alone does not stop a script from
navigating its own frame to an external URL. Parent frame policy also constrains
that navigation. The native adapter uses the platform's actual custom protocol
URL; on Windows it is `http://nexa-ui.localhost`.

Wry 0.55.1 on Windows injects initialization scripts into subframes regardless
of its `for_main_frame_only` setting. Nexa therefore replaces Tauri's default
invoke transport through its supported `invoke_system` interface with the
pinned transport plus an early main-frame/preview-origin guard. The guard runs
before constructing the key-bearing closure. Tauri helper objects may still
exist in a subframe, but no authenticated IPC transport is installed there.
Native acceptance verifies both denial from generated content and a successful
main-frame call; JavaScript presence checks alone are insufficient.

Stop, source changes and unmount release the stored preview and remove the
executing frame. Late creation responses are immediately released after a
stop/unmount. URLs expire after 30 minutes; remote device revocation also removes
its stored previews. Each preview is limited to 512 KiB and the store admits 24
concurrent instances. History stores source, not these temporary URLs or the
decision to run scripts. Restoring a conversation starts with an inert preview.

HTML must be self-contained browser JavaScript. JSX/TSX, npm dependencies,
relative files and remote libraries require an appropriately built local file
and the existing [Browser Workspace preview](local-html-preview.md).

## Compatibility and migration

Existing Markdown, Mermaid, messages and local HTML assets need no migration.
This is a breaking change to remote inline HTML: it now uses the same explicit
Run flow as desktop, and CDN scripts/styles/images are no longer fetched by that
sandbox. Move dependent artifacts to the explicit local-file preview route or
bundle their required resources into the document.

`nexa-ui` is a small Nexa format. It does not claim A2UI or MCP Apps wire
compatibility. A future adapter should translate to the existing presentation
model and preserve its authority, lifecycle and history contracts.

## Design sources

- [Google Generative UI research](https://research.google/blog/generative-ui-a-rich-custom-visual-interactive-user-experience-for-any-prompt/)
  motivates generation instructions, tooling, validation and useful interaction.
- [A2UI](https://a2ui.org/) and its
  [versioned protocol](https://github.com/a2ui-project/a2ui/blob/ce03d005c0575ff6708da583d9e7658a34df2d5d/specification/v0_9_1/docs/a2ui_protocol.md)
  inform the allowlisted component catalog, explicit versions and client rendering.
- [OpenAI UI guidance](https://developers.openai.com/plugins/concepts/ui-guidelines)
  and [MCP UI integration](https://developers.openai.com/plugins/build/chatgpt-ui)
  inform small in-conversation surfaces and host-controlled iframe isolation.
- [OpenAI Intelligent UI](https://openai.com/index/gpt-6-for-everyone/)
  describes streaming native components and compilation during generation.
  This is a design reference, not a public drop-in runtime that Nexa can claim
  to reproduce by changing its model selection.
- [pi extensions](https://github.com/badlogic/pi-mono/blob/4270522473e677ddba7b093caa0064a3df51d459/packages/coding-agent/docs/extensions.md)
  separate execution from rendering, while
  [pi skills](https://github.com/badlogic/pi-mono/blob/4270522473e677ddba7b093caa0064a3df51d459/packages/coding-agent/src/core/skills.ts)
  support progressive authoring guidance.
- [Codex app-server](https://github.com/openai/codex/blob/a9b0815167c27ab13719dd12d13b63e02b70d221/codex-rs/app-server/README.md)
  preserves UI metadata in history. Nexa reconstructs its inline views from the
  durable transcript rather than persisting live iframe grants.
- [Hermes cron tools](https://github.com/NousResearch/hermes-agent/blob/aefffa077f0f964596d54385ab91e011606e1596/tools/cronjob_tools.py)
  inform the single action-based tool over Nexa's existing scheduler; see
  [Scheduled tasks](SCHEDULED_TASKS.md#agent-tool).

## Verification

`npm run test:chat-ui` checks the actual format contract and streaming fallback.
The browser acceptance suite is
[`chat-generative-ui.spec.ts`](../../apps/desktop/e2e/chat-generative-ui.spec.ts):
desktop/mobile layout, filtering, sorting, keyboard tabs, history reconstruction,
inert streaming, explicit execution, revocation, and blocked cross-frame/storage/
network/navigation access. Remote socket tests verify the actual served headers,
owner revocation and both iframe routes. The opt-in native WebView2 test
`native_chat_ui_sandbox_has_scripts_but_no_parent_storage_or_ipc` checks the real
custom protocol and Tauri script-injection boundary. Browser mocks alone are not
native acceptance.
