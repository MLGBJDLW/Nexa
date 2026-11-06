# Native mobile application

[Guide index](README.md) · [Documentation home](../README.md)

Nexa Mobile `0.3.0` is a native Android application in
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
2. Choose **This phone**, **Connect computer**, or **Set up later** on first launch.
   For **This phone**, open phone settings and add a provider/model and API key,
   or pair with Desktop 0.18 or later to synchronize saved connections. The model catalog is shared with desktop. Enable task
   notifications so progress and **Stop** remain reachable when another app opens.
3. Ask the agent to create or edit a file. Phone tasks use the existing scoped
   file tools; the folder action shares workspace outputs through Android's
   chooser with a temporary content grant.
4. For **My computer**, enable public Phone access on the computer, then scan
   its QR code with the in-app scanner or paste its complete HTTPS pairing link.
   The scanner works without Google Play Services. The link includes both a one-use code and
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
click, text, scrolling, back/home and installed-app search/launch. Android 11 and
later also support screenshots and coordinate taps using the captured image's
pixel coordinates. Captures are resized before entering the shared vision/tool
pipeline; use a model that accepts images. Targeted input and Back require
the current observation ID and check the foreground package and target again.
Home and explicit app launches remain available without an observation ID when
an app temporarily exposes no accessible UI. Read operations wait briefly for
transitions and refresh Android's accessibility cache; recovery never replays a send.
Each action returns a new observation. Text input is read back and reported as
verified only when it matches. Android 13+ also supports native editor input when
an app rejects accessibility text replacement. Coordinate taps revalidate app
pixels, target controls, rotation and foreground identity; model latency alone
does not expire an unchanged screen. Changed targets return fresh evidence
without performing the old action. Permission loss and locked/unavailable screens
stop input.

With Desktop 0.19 or later, open phone settings and explicitly enable **Allow this
computer to control this phone** beside the paired computer. The desktop's
`phone_control` tool can then list devices and use the same native adapter.
Only one computer or local phone task owns input at a time. Existing Phone access
routes, including public HTTPS, carry this traffic; no inbound phone port is
required. Stop, revocation and process restart disable control. A temporary
reconnect retains the enabled session, cancels outstanding requests and never
automatically repeats a click, text entry or send whose outcome is uncertain.

Phone settings distinguish a disabled service, an enabled service that has not
connected, notification access, battery restrictions and a locked/asleep screen.
On Android 13 and later, a sideloaded APK can be subject to **Restricted settings**.
Open Nexa's system App info, use **Allow restricted settings** in the overflow
menu when offered, then return to Accessibility and enable Nexa. Samsung and
other vendors may place this control differently. Device policy can prohibit
the change; the app cannot bypass that restriction. Settings provides the relevant
system pages and vendor-aware guidance. If a service was reclaimed, toggle it
off/on and review the app's battery restrictions. No extra permission can
guarantee survival of force-stop or vendor background restrictions.

Actions use the existing approval path. Approval notifications let the user
approve once or deny without switching away from the target app. The task's
persistent notification provides Stop silently. Routine progress does not alert;
approval requests and a final reply received while Nexa is in the background use
the attention channel. A foreground service and a task-scoped
wake lock support user-started work. Phone control keeps an already-active screen
visible during its task but never unlocks or wakes a deliberately locked screen.
An optional draggable assistant bubble opens Nexa; hold it to reveal Stop.
It uses the enabled accessibility service's overlay and requires no separate
draw-over-other-apps permission. The app does not promise survival of a
force-stop, reboot, vendor power restriction or permission revocation.

The distribution target is a signed GitHub APK. This is not a claim of Google
Play acceptance for autonomous accessibility-based control. iOS and native
HarmonyOS need their own platform adapters, signing and device validation;
the Android APK does not establish support for either platform. Secure system
screens, password input and apps that prohibit capture remain protected.

## Chat and assistant capabilities

Phone tasks share desktop model discovery and searchable provider/model choices,
web search and URL fetching, image attachments, follow-up suggestions and saved
thinking history. Long thinking blocks load on demand. Choose **Ask** or
**Allow all** beside the composer. When supported by the host, the Plan icon switches between planning and execution without opening attachments; Plan mode still excludes mutating tools even
when Allow all is selected. Each conversation retains its own model and reasoning
intensity while connection credentials can continue to synchronize. Supported
models reason by default; the picker offers intensity controls.

Interactive chart requests prefer inline `nexa-ui` components. Completed answers
have local follow-up choices when the model omits valid suggestions. These choices
are inert until the user selects them.

To dictate into a draft, press and hold the phone message field and release when
finished. The microphone button is also available. Review or edit the transcript
before pressing Send. Phone settings independently select the dictation and read-aloud service. System
dictation uses an installed Android recognizer or an on-device engine. Empty
recognition results and activity interruptions are explicit errors, and first-use
permission results wait until the activity resumes. A failed system service can
fall back once to an available on-device engine. System read-aloud uses Android TTS.

Alternatively configure a provider, model and endpoint for each direction. Leave the speech key blank to reuse a compatible saved phone connection, or set a separate key to override it.
Custom dictation records mono PCM16 at 16 kHz in the foreground, then calls the
same complete-file transcription adapters as desktop. Tap Stop to submit, or
Cancel while transcribing to discard the pending result. Recordings are bounded
to 60 seconds and are not saved as chat files. Supported presets include OpenAI,
Groq, Qwen file transcription, SiliconFlow, Mistral, Google, Meta and compatible
custom endpoints; desktop-only executable and realtime-only input adapters are
not offered. Custom read-aloud uses shared provider synthesis with native Android
playback, without putting audio or credentials in the WebView. Saved speech keys
use the Android-keystore-wrapped settings key and remain local to this phone.

Both modes request microphone permission only when needed. Tests and system
configuration links are available in phone settings. Language preferences survive
restart. Read-aloud excludes tool output, thinking and suggestion metadata.
Stop/cancel and activity teardown release speech sessions. Live voice
conversation and desktop-only terminal/browser automation are not local phone tools.
Opening another app and interacting with its visible controls is supported;
completion, such as sending a message, must be verified from a fresh observation.

The optional floating assistant snaps to either edge after dragging and restores
its position. Releasing near the center produces a quicker, larger rebound; at the edge it settles gently. Idle floating pauses while suspended and honors Android's animation setting.
Storage settings show measured database, chat, knowledge, file, model and cache
categories. The conversation manager selects across pages for batch archive or
confirmed deletion. Running tasks must be stopped first. Archiving changes
visibility; reclaiming SQLite's reusable pages is a separate compaction action.

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
Continuation restores the original connection, selected model and execution
policy; a renderer restart cannot change a Plan task into a write-capable task.

## Chat and web settings

A new phone chat stays a local draft until its first send. Repeated New taps do
not create empty database rows. The composer uses one Send/Stop control; the
conversation drawer reveals Archive progressively as you swipe left. The archived
list supports Restore and confirmed deletion across all pages. Chats restored or
already removed by another client are skipped. Running-task checks still apply. Remote desktop chat
management requires the advertised `chat-management-v1` capability; bulk archived
deletion requires `chat-delete-archived-v1`. Completed replies keep the final
answer visible, with thinking and intermediate tool activity available on demand.
The phone generates an initial chat title without replacing a manual rename.

Phone settings support search and collapsible sections. Web settings use a touch layout of the shared web search panel, including provider-native
search, provider ordering and DeepSeek Search & Fetch. Native requests share the
core search/fetch adapters. Saved keys are masked in the WebView and encrypted
with a random settings key wrapped by Android Keystore; existing desktop
ciphertext remains compatible. Connection sync still covers model connections
only, so web-search credentials remain local to the phone. An explicit **Use this
connection for Search & Fetch** action can reuse an official DeepSeek connection's
key and prioritize its Search/Fetch API; hosted DeepSeek models do not qualify.

Markdown tables, code and math scroll inside their own containers. Narrow-screen
charts place labels and values above the bars; tabs and table filters remain
touchable. Mermaid can switch between fit and scrollable original size. Static
HTML previews constrain fixed-width layouts to the frame's viewport while keeping
the existing script/network sandbox. Storage management uses phone bottom sheets.

## Continuous context

Mobile defaults to continuous context; desktop offers it as an experimental
context-management choice. The existing history checkpoint path archives and
verifies the complete window before replacing it, preserves current user
instructions and adds bounded excerpts of recent progress. It makes no extra
summary model request, starts at no more than 80% of the usable context budget,
and keeps the same conversation, run and device-control session. Exact older
constraints/results remain accessible through `context_history`. Mobile reloads
the durable projection rather than resurrecting the archived context.

Custom system prompts are retained in full, without the previous fixed 256 KB
ceiling or fractional policy truncation. Generated skill scaffolding still uses
remaining context capacity; a request beyond the selected model's real window
is rejected explicitly rather than silently cutting instructions.

Routine automatic checkpoints do not emit compaction UI messages. Approvals,
actions, cancellation and actionable failures remain visible. This mode uses
extractive evidence, not a background LLM summary; excerpts can omit details, so
exact evidence must be retrieved when needed. Explicit manual maintenance and
hard-overflow safety retain their existing guarded paths.

## Conversation controls and scheduled tasks

Open **Scheduled tasks** in the conversation drawer to create a five-field cron job in an IANA timezone. The phone and each paired computer keep separate schedules. Jobs deliver replies to the chat where they were created. The editor supports previewing the saved next run, editing, pause/resume, deletion, occurrence history, and approval decisions. On the phone, local jobs are checked while Nexa is open and catch up on return. On a paired computer, the desktop scheduler continues while the phone is disconnected. Chat can also create jobs through the same automation tool. See [Scheduled tasks](SCHEDULED_TASKS.md) for occurrence, retry and overlap behavior.

Type during a running turn to steer it, or leave the composer empty to stop it. On Android the microphone expands into a live input-level meter. The pairing scanner follows the system rotation preference. Swipe an active chat left to expand its archive button. In the archive, swipe right to restore or left to confirm deletion. The archive heading also has a confirmed bulk-delete action.

## Build and release

Use the commands and prerequisites in the [application README](../../apps/mobile/README.md).
The [Mobile workflow](../../.github/workflows/mobile.yml) builds Android separately
from desktop. PRs validate the mobile frontend and version contracts without
installing an Android toolchain or packaging an APK. Content-verified Release
Please metadata updates skip the redundant mobile frontend build. Full signed
Android builds run only for a `nexa-mobile-v*` tag or an explicit release-tag
dispatch. Shared core changes retain the desktop core checks. Native Android
compilation and signing remain required before mobile release publication.

The [Mobile Release workflow](../../.github/workflows/mobile-release.yml) maintains
its own release PR, configuration and version manifest inside `apps/mobile`.
Merge that reviewed PR to create the `nexa-mobile-v<version>` draft and start
Android delivery. The release PR synchronizes the mobile npm, Tauri, Cargo and
lockfile versions and generates an English changelog. It does not change the
desktop version or start desktop packaging. For a manual release, update those
same versions and `apps/mobile/.release-please-manifest.json`, write the English
mobile changelog, run `node scripts/sync-workspace-lock-versions.mjs --write`,
then merge and tag the reviewed commit.

The workflow resolves one immutable source SHA for checks and packaging,
validates version/tag identity and ancestry on master, builds the ARM64 APK, verifies the
signature against the committed public certificate fingerprint, and publishes
the APK, SHA-256 manifest and certificate receipt. Mobile releases use
`latest=false` so desktop updater discovery keeps its existing release identity.
A failed draft can be resumed through the workflow's `release_tag` input.
Its release body contains only the exact current mobile changelog entry, including
on a draft retry. Desktop publication removes dedicated mobile sections and omits
mobile-only pull requests from its generated release notes.

Desktop release-please excludes the `apps/mobile`, `docs` and `.github`
directories, so phone changes accompanied by documentation or CI changes do
not cause a desktop version bump. Mobile workflow changes still run the mobile
checks. Shared Rust runtime and imported desktop UI changes continue to validate
their actual consumers; shared changes can require releases of both products.
When releasing a shared-only mobile fix, include a mobile changelog/source change
so the independent mobile release stream records it.

This follows release-please's [directory-based commit exclusions](https://github.com/googleapis/release-please/blob/main/src/util/commit-exclude.ts),
[per-package manifest configuration](https://github.com/googleapis/release-please/blob/main/docs/manifest-releaser.md)
and [path-prefixed release outputs](https://github.com/googleapis/release-please-action#path-outputs).
File names cannot be used as exclusion directories. CI-only and documentation-only
commits do not themselves ship desktop application changes.

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
