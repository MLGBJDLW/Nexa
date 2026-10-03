# MCP resources and prompt templates

Use **MCP resources and prompts** from the chat command palette, More options,
or `/mcp-context`. The same browser is available under each enabled connector in
Settings. Select a connector to browse resources, URI templates, and prompts.

Select a resource or fill the string parameters of an advertised URI template, then read it through that
connector. Resource URIs are opaque MCP identifiers: Nexa does not reinterpret
them as local file paths or independently fetch their URLs. Select a prompt and
fill its declared arguments to inspect the returned messages. **Add text to draft**
appends the displayed text and source to the current draft without sending it.
Binary blocks remain available through the agent's resource-read tool and its
existing MCP result/attachment projection; the manual draft action adds text only.

## Agent access

The `mcp_context` tool lists connected servers and pages their resource, resource
template and prompt catalogs. Reading a resource or retrieving a prompt uses the
same connector lifecycle as ordinary tools, with explicit read approval under
Ask mode. Lists use the existing catalog and do not execute a server tool.
Resource reads require exact membership in the current connector catalog. For
template resources, `read_resource_template` accepts the exact advertised
`uri_template` plus string `arguments`; the host validates and expands RFC 6570
operators, including reserved expansions only when declared by that template.
Arbitrary caller-expanded URIs and undeclared arguments are rejected before RPC.
The expansion uses the [URI Template parser](https://docs.rs/uri-template-system/0.1.5/uri_template_system/struct.Template.html)
and has bounded input and output sizes. Prompt names and arguments are likewise
validated against the current catalog before RPC.
Prompt message roles remain inert content; they never become system messages.
Retrieved data carries connector provenance and is treated as untrusted evidence.

## Compatibility and lifecycle

- Nexa retains its negotiated 2025/2024 MCP transport compatibility. Adding content
  support does not claim support for a different handshake or wire protocol.
- Declared capabilities determine which catalogs are requested. Resource-only and
  prompt-only peers do not need a `tools/list` implementation. The historical tool
  fallback remains for peers that send an empty capability object.
- Discovery consumes all pages within bounded page, item, byte and time budgets.
  Repeated cursors and duplicate identities reject the incomplete catalog. An
  explicitly unsupported optional resource-template endpoint yields no templates.
  Resource, template and prompt catalog validation failures have separate status
  and diagnostics; a malformed content page does not remove verified server tools.
- Resource/prompt list-change notifications invalidate the catalog. Content reads
  use the current server; they do not return a stale cached resource body.
- Connector configuration, disablement, cancellation and authority are checked
  again after discovery and immediately before retrieval. A stale UI or model
  tool cannot silently target a replaced connector. Refresh and start a new turn
  when its authority changes.
- Binary/text projection follows existing MCP result limits. Unsupported blocks
  and truncation are surfaced as notices. Content never runs automatically.
- Controller-isolated execution retains its restriction on external MCP access.
