# Phone access

Phone access connects a browser to the Nexa desktop runtime. The computer owns
conversations, model credentials, tools, and Live sessions. Keep Nexa running
and the computer awake; this is not a separately hosted cloud agent.

## Pair a phone

1. Open **Phone access** near the bottom of the desktop sidebar.
2. Select **Enable and show QR code**. The default options enable encrypted
   local-network access and automatic public access.
3. Scan the QR code in the phone's browser. Confirm that the link belongs to
   this Nexa installation, then complete pairing.
4. Open an existing conversation or send a message. The desktop page shows the
   saved device and whether it is online.
5. Use **Add another device** for another pairing, or **Revoke access** to remove
   a device's authority.

Pairing codes expire after three minutes, are single-use, and permit a bounded
number of failed attempts. Create a new code from the desktop after expiry or
lockout. Pair only your own devices: a paired device can read conversations and
authorized file previews, start agent work, answer questions, and approve
individual pending actions.

**Stop remote access** stops listeners and managed public routes. Exiting Nexa
does the same. Preparation can be cancelled. Nexa does not automatically edit
the firewall, install an operating-system service, or enable an SSH server.

## Select a route

Stop remote access before changing saved connection options, then start it again.

| Route | Requirements and behavior |
| --- | --- |
| Encrypted LAN | Default port 8791 over TLS. The phone must reach the computer and trust this installation's certificate. |
| Automatic public access | Tries localhost.run and Pinggy, with Cloudflare as an additional route. Readiness is checked end to end; selection is based on reachability, not UI language or assumed geography. |
| Explicit public provider | Advanced options can pin localhost.run, Pinggy, or Cloudflare instead of automatic selection. |
| Fixed HTTPS origin | Configure your own tunnel/reverse proxy to `http://127.0.0.1:8790`, then enter its HTTPS origin without a path, query, or credentials. |
| Existing SSH server | Use a phone SSH client to forward phone `127.0.0.1:8790` to computer `127.0.0.1:8790`, then open `http://127.0.0.1:8790` on the phone. Account, server, and network access must already exist. |

A managed SSH tunnel is an outbound helper connection, distinct from exposing
your own SSH server. localhost.run and Pinggy need an available OpenSSH client.
Nexa uses isolated SSH configuration and managed known-host state rather than
offering the user's personal keys or SSH agent.

The managed Cloudflare helper is downloaded from a pinned official release and
verified against its bundled SHA-256 metadata. Automatic helper download is
implemented for Windows x64 and Linux x64/ARM64; use another supported route or
a fixed HTTPS origin where that helper is unavailable. A verified cached helper
can start without re-querying the release API.

Free public routes can expire, change addresses, or be unavailable. Nexa probes
HTTPS identity, POST requests, RPC admission, and the WebSocket upgrade before
advertising a managed route as ready, and retries failed managed connections.
A helper printing a URL is insufficient. After a long offline period or a lost
temporary address, scan a fresh pairing link from the desktop.

Cloudflare describes Quick Tunnels as a testing/development service with no
uptime guarantee. Use your own fixed route when a stable address is required.
See [Cloudflare's Quick Tunnel limits](https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/do-more-with-tunnels/trycloudflare/).

Public providers carry remote traffic. Pairing protects access to Nexa; it does
not remove the tunnel provider from the transport path.

## LAN certificate and phone permissions

On the phone, open connection settings and choose **Download LAN certificate**.
Install the public certificate as trusted on that phone. On iOS, also enable
full trust in Certificate Trust Settings. Allow local-network access if the
browser requests it. These are user-managed trust decisions; Nexa does not
install the certificate for you.

Each installation has its own certificate authority. The private signing key
stays on the computer; the phone downloads only the public certificate.
Without LAN trust, automatic selection can retain a reachable public HTTPS
route rather than downgrading to plaintext LAN traffic.

Windows may request firewall permission for the selected private network.
Different Wi-Fi isolation, firewall, VPN, or proxy rules can prevent LAN access.
The phone microphone/camera also needs a trusted secure context and browser/OS
permission. See [Voice and Live](LIVE.md).

## Chat, previews, and appearance

The phone can browse and page through desktop conversations, send/stop work,
answer supplemental questions, and allow or deny a single pending approval.
Requests use the same desktop launch and approval paths; model keys are not
sent to the phone.

Model connections and model choices come from the desktop resolver. Refresh
available models or enter a custom ID where supported. Dictation uses the
desktop's configured streaming speech recognition. Recognized draft text is
preserved if capture disconnects.

Phone appearance can follow the desktop or use its own selection. Authorized
theme assets and file previews are served through the authenticated remote
surface. A preview's successful opening is not document validation.

Chat submissions keep a request ID across retries to avoid launching the same
task twice during a network change. Event sequence numbers support incremental
recovery. Long history is paginated, and reconnect does not require sending the
complete trace or original image data as one payload.

## Live

The phone has separate Chat and Live entries. Live uses the same backend as the
desktop, with browser-provided microphone, camera, and screen input where
available. Network switching pauses capture; session leases bound recovery.
Stopping preserves text observations and summaries instead of raw audio/video.

See [Voice and Live](LIVE.md) for supported connection modes, input ordering,
capture cleanup, and record limits.

## Troubleshooting

| Symptom | Next step |
| --- | --- |
| Pairing expired or failed too often | Generate a new code on the desktop |
| Computer identity mismatch | Stop using that link and scan the QR shown by the intended desktop |
| No public route is ready | Inspect provider diagnostics; check OpenSSH/helper download and network access, or configure a fixed origin |
| LAN does not connect | Check certificate trust, browser local-network permission, private-network firewall access, and Wi-Fi isolation |
| Microphone unavailable | Use trusted HTTPS/localhost and grant microphone access; configure desktop speech recognition for dictation |
| Reconnection never completes | Confirm Nexa is still running; check the latest route on the desktop and rescan if the saved temporary addresses are gone |
| Device was revoked | Pair again only if the desktop user intends to restore access |

## Implementation and verification

- [Remote crate](../crates/remote/src/lib.rs): authenticated transport and host interface.
- [Public-route manager](../crates/remote/src/public_access.rs) and
  [helper lifecycle](../crates/remote/src/tunnel.rs).
- [Desktop bridge](../apps/desktop/src-tauri/src/remote.rs) and
  [phone client](../apps/desktop/src/features/remote/remoteClient.ts).
- [Protocol tests](../crates/remote/tests/remote_protocol.rs),
  [setup regressions](../apps/desktop/e2e/remote-setup.spec.ts), and
  [phone regressions](../apps/desktop/e2e/remote-phone.spec.ts).

Run `cargo test -p nexa-remote` from the root and the relevant Playwright specs
from `apps/desktop`. Public-tunnel smoke tests are explicitly ignored by default;
physical phone permissions and real network/provider availability require
separate acceptance.
