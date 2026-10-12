---
name: visual-explanations
description: Visualize explanations, comparisons, data, processes, or simulations in chat. Use for diagrams, interactive tables, charts, timelines, what-if tools, and self-contained HTML demos; 可视化、交互图表、动态演示、流程图. Choose a visual only when it makes the answer easier to understand.
---

# Visual explanations

Choose the smallest surface that helps the reader. Keep the useful conclusion
and evidence in the surrounding answer so the result also works as plain text.

- **Mermaid:** static flows, relationships, sequences, and state machines. Use
  the existing `mermaid` fence and keep labels readable.
- **Structured UI:** metrics, sortable/filterable tables, signed bar charts,
  steps, or tabs. Read [the format](references/structured-ui.md), then emit one
  `nexa-ui` fence containing its versioned JSON. Nexa handles layout, theme,
  accessibility, and local interaction. Use JSON data, not executable expressions.
- **HTML:** a simulation, calculator, bespoke visualization, or interaction that
  the structured components cannot express. Read
  [HTML authoring](references/html.md), then emit a self-contained `nexa-html`
  fence. Static HTML/CSS appears during generation; the user can enable scripts
  with Run after generation finishes. Ordinary `html` fences remain code with
  an optional Preview button.
- **A deliverable with files/assets:** use existing file tools and `open_in_nexa`
  with the exact supporting assets. This opens Browser Workspace and retains the
  established observe/act/verify workflow. Inline UI is not a hosted website.

For `nexa-ui`, validate the JSON with
`node "<SKILL_DIR>/scripts/validate.mjs" <path>` when a Node 24 runtime is
available, or pass the JSON on stdin. This is the same validator used by the
frontend. Fix its field-specific errors before delivering. Templates are in
[examples](resources/examples.json). Put version and title before blocks so
complete blocks can appear while the rest is streaming. Close the fence.

Label quantities, units, time ranges, sources, and assumptions. Values must come
from user data, cited evidence, or clearly marked examples. Tabs, filters and
sliders change the local presentation; they do not execute tools or submit user
data. Operational actions such as saving schedules use the real `automation`
tool and its returned records, never a pretend button in a generated card.

Use a short takeaway after the visual. Check narrow layouts and the key
interaction when a browser is available. Report exactly what was validated:
format validation, rendered appearance and live data are separate checks.
If a requested capability needs a network call, file, host API, external library,
or unavailable runtime, use the appropriate existing tool or deliver a file;
the inline sandbox has no network, filesystem, clipboard, or Nexa tool bridge.
