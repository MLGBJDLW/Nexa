# Protocol Exits

Protocol exits let other agents or hosts call bounded Nexa capabilities. They
are not native plugins. They are host-owned interfaces with explicit source
scope, approval policy, and exported capability lists.

Status: [protocol_exports.rs](../crates/core/src/protocol_exports.rs) defines
maturity metadata. MCP server export is `Candidate`; ACP and A2A are `Design`.
The workspace does not ship a standalone `nexa-mcp-server` executable. These
definitions are not server startup instructions or proof of an active listener.

## Ordering

Nexa should add protocol exits in this order:

1. MCP server mode
2. ACP agent mode
3. A2A agent mode

MCP server mode comes first because MCP is already part of Nexa as a connector
client, and top agent products commonly use MCP as the tool execution protocol.
ACP and A2A should wait until the scoped MCP server export is tested.

## First Export Candidate

`nexa-mcp-server` is the first candidate protocol exit.

Initial exported capabilities:

- `search_knowledge_base`
- `retrieve_evidence`
- `list_sources`
- `get_document_info`

These are read-oriented capabilities that align with Nexa's local knowledge
strength. Write, shell, desktop automation, and connector tools should not be
exported until source scope, approval, and audit behavior are proven.

## Trust Requirements

Every protocol exit must:

- require explicit source scope
- require approval for any capability that can write, execute, access network,
  or cross a trust boundary
- expose a fixed tool/capability list by maturity stage
- keep credentials out of model-visible protocol payloads
- tag exported evidence with local or external trust metadata
- provide tests for protocol request validation and denied operations

## Non-Goals

Protocol exits should share scoped evidence and bounded workflows while
retaining the desktop assistant's trust model. The implemented
[paired phone surface](remote-access.md) uses its own authenticated typed
transport and is not an MCP, ACP, or A2A export.
