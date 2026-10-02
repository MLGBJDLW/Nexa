# MCP Connectors

MCP is Nexa's first external ecosystem lane. An MCP connector gives Nexa access
to tools exposed by an external process or remote service through the Model
Context Protocol.

MCP connectors are not native plugins. They run outside Nexa's core runtime and
are mediated by the connector host, tool approval policy, source scope, and
transport lifecycle.

## Connector Lifecycle

1. Configure
   - Choose a transport: `stdio`, `streamable_http`, or legacy `sse`.
   - Provide launch command, URL, arguments, environment, or headers as needed.
   - Keep credentials in connector config fields designed for secrets.
2. Test
   - Start or connect to the server.
   - Discover the server-defined tools.
   - Surface connection errors without enabling the connector silently.
3. Enable
   - Register discovered tools as MCP connector runtime tools.
   - Keep the connector disabled by default until the user enables it.
4. Use
   - Tool calls are keyed by server and tool identity.
   - High-risk calls still pass through approval policy.
   - Returned content is treated as external or mixed-trust evidence unless a
     connector-specific contract says otherwise.
5. Disable or delete
   - Disabling removes the runtime tools from discovery.
   - Deleting removes the stored connector configuration.

## Trust Model

MCP tools cross a trust boundary. Nexa should assume an MCP connector can:

- read data from outside the local knowledge base
- perform network operations
- mutate remote systems
- return prompt-injection content
- expose tool schemas that change between sessions

The connector host must therefore keep:

- server/tool identity in approval keys
- connection status visible
- discovered tools inspectable
- disabled connectors undiscoverable by the agent
- credentials out of model-visible context
- remote content marked as external unless explicitly grounded

## Product Language

Use "MCP connector" in user-facing UI and docs.

Use "MCP server" only when referring to the protocol endpoint, process, or
server-defined tool schema. This keeps the ecosystem model clear:

- Connector: the Nexa-managed external integration.
- Server: the MCP process or remote endpoint behind the connector.
- Tool: a server-defined callable capability exposed through the connector.

## Current Implementation

The runtime stores endpoint configuration as `McpServer` records. A tool's
canonical identity is the stable connector ID plus the exact server tool name.
Model-facing aliases use `mcp__<tool-label>__<identity-hash>` and are bounded to 63
ASCII characters. Display names, discovery order, case folding, punctuation and
another connector's installation cannot redirect a call. Existing package
ownership declarations are matched through a separate compatibility selector;
that selector is never an executable name or approval key.

An approval binds the canonical identity and a digest of the launch, endpoint,
environment and header configuration. Display renames retain approval identity;
changing that trust configuration requires a new grant. Legacy name-only and
wildcard grants remain stored for inspection but are not inherited by canonical
MCP calls. Credentials are hashed rather than exposed in permission keys. A
prompt carries one absolute 60-second deadline through its event, queue and
backend waiter; reconnecting the renderer does not restart that deadline.

Each connector owns its connection, cancellation epoch, RPC serialization and
catalog snapshot. Global manager and desktop registry-cache locks cover only
short in-memory reads or publications. Discovery admits at most four connectors
at a time; a slow connector does not lock other connectors' calls or settings
snapshots. Saving, enabling, testing or enumerating one connector targets that
connector. Tests use an isolated lifecycle and do not activate runtime tools.
Disabling or replacing configuration cancels outstanding discovery and prevents
its late result from becoming executable. Database-backed discovery and calls
recheck durable activation after waiting for I/O.

Tool catalogs follow every `nextCursor` before publication. A catalog is bounded
to 128 pages, 4,096 tools and 8 MiB; repeated cursors, duplicate exact names,
malformed pages, timeouts and failed pages leave the catalog incomplete. A
snapshot includes connection epoch, catalog revision, completeness and diagnostic
text. Registry assembly only reads snapshots and never performs discovery.
`notifications/tools/list_changed` invalidates and refreshes the affected
connector, with a 25 ms burst debounce. Stdio, legacy SSE and Streamable HTTP
readers observe notifications while RPC is idle; optional HTTP GET rejection
still permits POST RPC. A 60-second catalog TTL provides a fallback. A registry
handle cannot invoke a removed or changed definition after refresh. Transport
failure can recover the connection for a later call, but never automatically
replays the failed effectful call.

MCP output is retained as a versioned `mcpToolResult` artifact: text, image, audio,
embedded text/blob resources, resource links, structured content, metadata and
`isError` survive the model/display split and history persistence. Valid images
also enter the existing vision input path. Audio and other binary resources have
explicit model references and remain available in the artifact; resource links
are not fetched automatically. Business `isError` results do not reconnect.
Unsupported or invalid blocks have explicit notices. Results are bounded to 128
blocks, 4 MiB per binary block, 12 MiB total decoded binary, 512 KiB per text block,
1 MiB each for structured content and metadata, and 32 megapixels per image.
Declared image/audio MIME types are checked against signatures. External output
remains evidence and cannot grant instruction authority.

Advanced users can maintain the versioned `~/.nexa/connectors/mcp.json` file
(`NEXA_HOME` can override the `.nexa` root). Settings -> Extensions -> MCP
Connectors exposes the resolved path plus explicit Open and Reload actions. The
file is a declarative configuration source; startup and explicit Reload validate
the whole document before materializing its connectors into the existing
runtime store. Existing `mcp-connectors.json` content in app data is copied once
when the new target is missing; the legacy file is retained for rollback.

```json
{
  "version": 1,
  "connectors": {
    "local-docs": {
      "name": "Local Docs",
      "transport": {
        "type": "stdio",
        "command": "npx",
        "args": ["-y", "@example/docs-mcp"],
        "env": {
          "DOCS_TOKEN": "${env:DOCS_TOKEN}"
        }
      }
    },
    "remote-docs": {
      "name": "Remote Docs",
      "transport": {
        "type": "streamable_http",
        "url": "https://example.com/mcp",
        "headers": {
          "Authorization": "Bearer ${env:DOCS_TOKEN}"
        }
      }
    }
  }
}
```

Connector ids use letters, numbers, `.`, `-`, or `_`. Secret-shaped environment
variables and headers must use `${env:VARIABLE}` references; resolved values are
never written to the JSON or SQLite projection. A new file connector is disabled
until the user explicitly enables it. Reload preserves activation only when the
normalized trust-relevant configuration is unchanged; name, command, arguments,
URL, environment, or headers changing resets it to disabled. Invalid JSON
retains the last valid runtime projection and reports the parser line and column.

The JSON file does not carry trust receipts, tool approvals, health state, or
native code. Those remain host-owned state. File-backed connectors are edited in
the JSON rather than through the managed form.

Nexa's generic capability package loader can read connector metadata from
`.nexa/capabilities/*/capability.yaml`. A valid generic declaration looks like:

```yaml
id: github-mcp
name: GitHub
surface: connector
description: Connector package metadata for a separately configured MCP endpoint.
version: 1
permissions:
  read: true
  write: true
  network: true
```

Connector package metadata should describe setup, required credentials,
permissions, health checks, and safe default state. It should not load native
code into Nexa core.

The generic manifest does not launch or configure the transport. Actual command,
URL, environment, headers, and activation belong to the saved connector or the
versioned `mcp.json` declaration above. The `@example/docs-mcp` command is an
illustrative placeholder, not an installation recommendation.

Implementation: [MCP runtime](../crates/core/src/mcp),
[extension storage](ECOSYSTEM_ARCHITECTURE.md#user-owned-extension-home), and
[capability parser](../crates/core/src/capability_package.rs). Verify
disabled-tool discovery, configuration-change trust reset, secret references,
and invalid-file retention when changing connector behavior.
