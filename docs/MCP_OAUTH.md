# MCP OAuth sign-in

Remote MCP connectors can use an explicit browser sign-in instead of a pasted
Authorization header. In **Settings → Extensions → MCP connectors**, save and
enable the connector, open **OAuth sign-in**, save its OAuth settings, then choose
**Sign in**. The system browser opens after Nexa binds a loopback callback.
Cancel remains available while discovery or sign-in is pending.

Leave Client ID empty to use the server's dynamic native-client registration.
A pre-registered public client ID is supported. A real HTTPS client metadata
document can be entered as the client ID when the authorization server advertises
that feature; Nexa does not invent or host such a document. Confidential clients
requiring a client secret are not supported. Advanced settings can pin an issuer,
pin a resource identifier, or select a fixed loopback callback port. Without a
fixed port, the callback uses `http://127.0.0.1:<ephemeral-port>/oauth/callback`.
Dynamic registration is repeated for that exact callback on each sign-in.

## Authority and storage

- Public configuration, status, granted scopes, expiry and a random credential
  reference are stored in SQLite. Tokens are kept in Windows Credential Manager
  with local persistence, macOS login Keychain, or Linux Secret Service.
- Linux requires an available, unlocked Secret Service. There is no plaintext or
  machine-derived-key fallback. Vault operations run on one dedicated thread;
  temporary access failures remain recoverable without restarting Nexa.
- Tokens are not placed in `headersJson`, configuration exports, model tools,
  browser-renderer responses or diagnostic response bodies. OAuth settings are
  local to this installation; the connector JSON declaration does not export them.
- Endpoint/header changes, disabling, deleting, changing OAuth configuration,
  reconnecting and disconnecting advance the authorization epoch. Old callbacks,
  tool grants and catalogs cannot acquire the new authority. Token rotation alone
  keeps the same grants. Scope changes invalidate the former authority.
- `Authorization` and `Cookie` headers cannot be combined with OAuth. Extra scopes
  require an explicit settings edit and sign-in. The client does not interpret
  an unverified JWT as an account identity.
  Initial sign-in and renewal both reject scopes outside the requested set. If
  a service grants default scopes, enter those intended scopes explicitly before
  signing in; an empty scope setting does not approve an unknown default grant.

Native ACP agents own their MCP clients and credentials. Nexa-managed OAuth is
available through Nexa's managed MCP transport, including host-tool runtimes;
selecting one of these connectors for native ACP returns an actionable error.
Configure that native agent's own OAuth flow instead. Nexa never exports a
refresh token or a short-lived bearer token to an ACP process.

## Discovery and requests

Discovery follows the Bearer `resource_metadata` challenge, then path-specific
and root well-known metadata. The protected resource is bound to the candidate
resource identifier, and the authorization issuer must match exactly. An explicit
resource pin can accommodate same-origin parent-resource deployments. Metadata
and token requests have bounded bodies, pinned public DNS resolution, no redirects
or inherited connector headers. HTTP is allowed only for the explicitly configured
loopback connector's origin. Private-network authorization servers are not accepted.

Every login uses independent secure random state and PKCE S256. Missing S256
support, state mismatch and issuer mismatch cannot exchange a code. A login has a
five-minute callback deadline; stopping the app cancels pending logins on recovery.
Callbacks do not expose a token or reflect remote error text in their browser page.

Before a request, an expiring access token is renewed with a per-connector lock.
A rotated refresh token replaces its predecessor through an immutable vault record
and an atomic database reference update. A response without expiry does not display
a fabricated expiry date. Missing refresh tokens require a later explicit sign-in.
`invalid_grant` and temporary server failures have distinct status values.

An actual HTTP 401 allows one renewal/retry for initialization, catalog listing,
resource reads and prompt reads. **`tools/call` is never transparently reposted**,
including after 401, session loss, timeout or a server error. A new user/model
invocation can attempt renewal after an authorization rejection. A 403 scope
challenge appears in OAuth status for review and never silently upgrades scopes.
OAuth credentials cannot follow a legacy SSE message endpoint to another origin.

Disconnect invalidates local authority first. When a revocation endpoint exists,
Nexa also requests remote revocation and reports its result separately. A remote
failure does not restore local access. Pending vault deletions are durable and can
be retried after unlocking the system credential store.

This feature retains Nexa's existing MCP 2025 wire protocols. It does not claim
the different 2026 wire transport merely by implementing newer OAuth safeguards.

## Verification

Core HTTP fixtures cover discovery, registration, the callback/code exchange,
PKCE and issuer binding, token rotation, concurrent renewal, cancellation,
late callbacks, scope changes, remote revocation failure and the write-call
non-replay boundary. An opt-in native vault test writes, reads and deletes only
random temporary fixture credentials. The browser test covers the visible settings
flow, cancellation while discovery is pending and rejection of late UI responses.
These tests do not establish compatibility with every third-party account or issuer.
