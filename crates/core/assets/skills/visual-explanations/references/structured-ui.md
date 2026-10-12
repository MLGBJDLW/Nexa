# Structured chat UI v1

Emit strict JSON in a `nexa-ui` fence. Top-level keys: `version: 1`, `title`,
optional `summary`, and `blocks`. Unknown fields and component types are errors.
All text is plain text, with no embedded Markdown, HTML, URLs or tool actions.

Each block has one of these shapes:

| type | Fields |
| --- | --- |
| `text` | `text: string` |
| `metrics` | `items: [{label: string, value: string or number, detail?: string}]` |
| `table` | `title?: string`, `columns: string[]`, `rows: (string or number or null)[][]` |
| `bars` | `title?: string`, `unit?: string`, `items: [{label: string, value: finite number}]` |
| `steps` | `items: [{title: string, detail?: string, status?: pending or current or complete}]` |
| `tabs` | `tabs: [{label: string, blocks: block[]}]` |

Table rows must match the column count. Numbers sort numerically; null displays
as an em dash. Bar charts use a common zero baseline and accept signed values.
Keep the visual small enough to read in chat; deliver large datasets as files.
The executable [contract](../resources/chat-ui.ts) defines transport and rendering
capacity limits and is also used by the bundled validator.

```nexa-ui
{"version":1,"title":"Delivery overview","summary":"Example data, three milestones.","blocks":[{"type":"metrics","items":[{"label":"Reviewed","value":"8 / 10"},{"label":"Open findings","value":2,"detail":"Awaiting verification"}]},{"type":"steps","items":[{"title":"Research","status":"complete"},{"title":"Validation","status":"current"},{"title":"Release","status":"pending"}]}]}
```

Use `complete` only for confirmed results. A generated visual does not authorize
or prove any external action. This is Nexa's small format, not the A2UI or MCP
Apps wire protocol; do not paste their payloads into a `nexa-ui` fence.
