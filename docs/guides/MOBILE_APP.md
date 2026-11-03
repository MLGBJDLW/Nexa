# Native mobile application

[Guide index](README.md) · [Documentation home](../README.md)

Nexa Mobile starts at version `0.1.0` with a native Android application in
[`apps/mobile`](../../apps/mobile/README.md). It has two explicit execution targets.

| Target | Runtime and credentials | Files and history |
| --- | --- | --- |
| This phone | Shared Rust agent core hosted on Android; use a saved or synchronized model connection | Android app-owned workspace and SQLite conversation/run ledger |
| My computer | The paired computer's existing agent and provider connections | Remain on that computer; the phone receives chat, approval and preview projections |

Phone execution does not require a running Nexa computer. It still needs network
access to its configured model unless the user supplies a model server on the
phone. This release does not bundle an offline model, Linux environment or shell.
Cloud provider URLs require HTTPS; loopback HTTP is permitted for a model server
on the phone itself. A pairing connection always requires trusted HTTPS.

## Setup and use

1. Install the signed ARM64 APK from the matching `nexa-mobile-v*` release.
2. For **This phone**, open phone settings and add a provider/model and API key,
   or pair with Desktop 0.18 or later to synchronize saved connections. The model catalog is shared with desktop. Enable task
   notifications so progress and **Stop** remain reachable when another app opens.
3. Ask the agent to create or edit a file. Phone tasks use the existing scoped
   file tools; the folder action shares workspace outputs through Android's
   chooser with a temporary content grant.
4. For **My computer**, enable public Phone access on the computer, then paste
   its complete HTTPS pairing link. The link includes both a one-use code and
   the expected computer identity. Select a saved computer to reconnect.

API keys and per-host pairing tokens are encrypted with a key held by Android
Keystore. They are not stored in WebView local storage or returned by the native
state API. Changing the provider or endpoint requires the corresponding key;
a saved key is retained only for the same account endpoint. Android backup is
disabled so encrypted records cannot be restored without their keystore key.
Forgetting a computer removes this phone's saved token; revoke the device on the
computer to invalidate any other saved copy.

## Model connection synchronization

Pairing the native app with Desktop 0.18 or later grants bidirectional sharing of
portable model connections and API keys. Existing pairings need a fresh pairing
grant; browser companion tokens do not gain credential export. Sync runs after
pairing, reconnecting, foregrounding, and connection changes. **Sync now** in phone
settings retries explicitly. Both devices retain their own active/default model,
workspace, tools and approval policy. Chats and files are not copied between devices.

Connections use stable shared identities and compare the last synchronized
revision. Simultaneous edits become separate named connections, preserving both
versions. OAuth sessions, external agent runtimes, device-local endpoints and
other device-specific authentication stay on their original device and remain
usable through computer mode. Portable connections require cloud HTTPS endpoints.

Keys cross only the authenticated native sync route and stay in the existing
encrypted desktop and Android stores. The renderer receives connection metadata
and sync counts, never saved keys. Removing a connection on the phone removes its
local copy and suppresses reimport. Deleting a desktop connection prevents the
phone from recreating it on that desktop; the phone keeps its independent copy.
Forgetting or revoking pairing stops future synchronization but does not erase
keys already copied to another device. Remove those copies or rotate the provider
key when needed.

## Optional phone control

Enable Nexa's accessibility service explicitly in Android settings. The adapter
reads visible controls on demand, omits password fields, and exposes observation,
click, text, scrolling, back/home and installed-app launch. Each action requires
the current observation ID and checks the foreground package and target again.
A successful dispatch is not reported as verified completion: the agent must
observe and verify the result. Changed targets, expired observations, permission
loss and locked/unavailable screens stop the action.

Actions use the existing approval path. Approval notifications let the user
approve once or deny without switching away from the target app. The task's
persistent notification provides Stop. A foreground service and a task-scoped
wake lock support user-started work; the app does not promise survival of a
force-stop, reboot, vendor power restriction or permission revocation.

The distribution target is a signed GitHub APK. This is not a claim of Google
Play acceptance for autonomous accessibility-based control. iOS and native
HarmonyOS need their own platform adapters, signing and device validation;
the Android APK does not establish support for either platform. Screen capture,
local voice input and local image attachments are capability-gated out of this
first release. The browser companion retains its existing voice/Live features.

## Runtime and recovery

The local host uses `ExecutorAgentSession`, the shared durable launch transaction,
run-event outboxes and cancellation token. Both phone and computer modes use the
same chat timeline, bounded message/run replay and approval UI. The native bridge adapts
transport; it does not implement another model/tool loop. Phone file access is
confined to its app-owned workspace. One local task runs at a time to avoid two
agents competing for the foreground app.

Renderer reload and foregrounding replay the durable ledger. Interrupted tasks
become explicit recovery checkpoints after process restart; they do not silently
repeat a possibly completed phone action. **Resume task** starts the shared
checkpoint continuation path, retaining the original run identity. An explicit
Stop remains available while the task is active.

## Build and release

Use the commands and prerequisites in the [application README](../../apps/mobile/README.md).
The [Mobile workflow](../../.github/workflows/mobile.yml) builds Android separately
from desktop. Mobile-only PRs keep the desktop repository/metadata checks and
use the Android workflow for their native build. Shared core changes still run
both relevant platform gates. No emulator or phone toolchain is added to the
ordinary desktop jobs.

To release, update the mobile package, npm lockfile, native Cargo package and
Tauri versions together, write the English mobile changelog, and synchronize
workspace lock metadata with `node scripts/sync-workspace-lock-versions.mjs --write`.
Merge the reviewed change and push `nexa-mobile-v<version>` for that commit.
The workflow validates version/tag identity, builds the ARM64 APK, verifies the
signature against the committed public certificate fingerprint, and publishes
the APK, SHA-256 manifest and certificate receipt. Mobile releases use
`latest=false` so desktop updater discovery keeps its existing release identity.
A failed draft can be resumed through the workflow's `release_tag` input.

Signing requires repository secrets `NEXA_MOBILE_KEYSTORE_B64`,
`NEXA_MOBILE_KEY_PASSWORD` and `NEXA_MOBILE_KEY_ALIAS`. Keep the private keystore
and its password outside the repository; retain the same signing identity for
upgrades. PR jobs never load release signing secrets.

## Acceptance boundaries

Compilation proves the dependency and packaging boundary. Browser tests prove
UI projections. Emulator runs can exercise the native bridge, keystore, task
notifications, owned-file tools and accessibility behavior. None of these proves
physical-phone battery behavior, every vendor WebView, real account entitlement
or paid model inference. Record those separately with the exact tested build.

Implementation owners: [local runtime](../../apps/mobile/src-tauri/src/local.rs),
[native remote transport](../../apps/mobile/src-tauri/src/remote.rs),
[Android device adapter](../../apps/mobile/src-tauri/gen/android/app/src/main/java/com/nexa/mobile/PhoneAccessibilityService.kt),
and [shared chat](../../apps/desktop/src/features/remote/RemoteChat.tsx).
