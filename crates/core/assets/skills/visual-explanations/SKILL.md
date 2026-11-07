---
name: visual-explanations
description: "Interactive charts and visual explanations inside the conversation: sortable tables, comparisons, timelines, diagrams, what-if calculators, and simulations. Use for 可互动图表、交互图表、可视化、动态演示、流程图、时间轴、时间线. Deliver inline nexa-ui or nexa-html components by default."
---

# Visual explanations

Deliver the requested visual inside the assistant reply. The final answer must
contain a closed `nexa-ui`, `nexa-html`, or `mermaid` fence with the actual visual
payload. For a requested timeline, include dates, events and relationships in a responsive timeline inside the chat; use `nexa-html` for a custom visual layout. A generic checklist does not replace a timeline. A file path or a description of a chart does not deliver an inline chart.
Choose the smallest surface below; keep its takeaway and evidence nearby.

- **Mermaid:** static flows, relationships, sequences, and state machines. Use
  the existing `mermaid` fence and keep labels readable.
- **Structured UI:** metrics, sortable/filterable tables, signed bar charts,
  steps, or tabs. Read [the format](references/structured-ui.md), then emit one
  `nexa-ui` fence containing its versioned JSON. Nexa handles layout, theme,
  accessibility, and local interaction. Use JSON data, not executable expressions.
- **HTML:** a timeline with dates and eras, a simulation, calculator, bespoke visualization, or interaction that
  the structured components cannot express. Read
  [HTML authoring](references/html.md), then emit a self-contained `nexa-html`
  fence. Static HTML/CSS appears during generation; the user can enable scripts
  with Run after generation finishes. Ordinary `html` fences remain code with
  an optional Preview button.
- **An explicitly requested file, website, or project with external assets:** use existing file tools and `open_in_nexa`
  with the exact supporting assets. This opens Browser Workspace and retains the
  established observe/act/verify workflow. Inline UI is not a hosted website.

For a small interactive comparison, start directly with this data-only pattern
(replace the illustrative data and labels with the user's data):

```nexa-ui
{"version":1,"title":"Illustrative comparison","summary":"Example values, not measured results.","blocks":[{"type":"tabs","tabs":[{"label":"Chart","blocks":[{"type":"bars","unit":"units","items":[{"label":"A","value":12},{"label":"B","value":8}]}]},{"label":"Explore data","blocks":[{"type":"table","columns":["Option","Value"],"rows":[["A",12],["B",8]]}]}]}]}
```

The table already supports filtering and sorting, and the tabs are interactive.
For sliders or a custom interactive plot, use the `nexa-html` example in
[HTML authoring](references/html.md). These inline components run in the chat on
desktop and phone; they do not need a saved HTML file or a local server.

For larger `nexa-ui` payloads, validate the JSON with
`node "<SKILL_DIR>/scripts/validate.mjs" <path>` when a Node 24 runtime is
available, or pass the JSON on stdin. A temporary validation file is optional;
the final delivery remains an inline fence. This is the same validator used by the
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
