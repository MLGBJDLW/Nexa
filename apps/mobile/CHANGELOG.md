# Changelog

## [0.3.7](https://github.com/MLGBJDLW/Nexa/compare/nexa-mobile-v0.3.6...nexa-mobile-v0.3.7) (2026-11-13)

### Improvements

- View and control an explicitly shared Windows window from paired phone chat, with touch-friendly controls, text input, keyboard actions, scrolling, zoom and Stop.
- Open screen assistance in a compact floating image panel with a question field and optional continuous preview.
- Translate a frozen screenshot directly over its original text regions. Tap a translated region to briefly reveal the original; submitted questions and answers remain available in normal conversation history.
- Show a non-interactive animated border while Nexa applies phone input. Assistant feedback is hidden during capture, and preview frames remain in memory.
- Let an authorized desktop view and control the Android screen, including focused text entry while keyboard pixels remain masked.
- Persist phone action receipts and uncertain outcomes without replaying input after interruption.
- Keep storage scans independent of conversation and model loading, reduce repeated web requests, and refresh verified provider/model routes while preserving saved selections.

### Screen-assistance availability

Screenshot assistance requires Android 11 or newer, accessibility permission and an unobstructed app with a complete, non-sensitive accessibility tree. Protected content is excluded; pixels outside the observed app and keyboard pixels are masked. Translation requires a suitable vision model, and translation quality and placement depend on that model.

Paired screen views refresh approximately once per second after each capture completes. Translation applies to the submitted frozen image rather than continuously translating every preview frame. Closing the assistant cancels pending requests; screen-only assistant requests cannot call tools.

### Features

* **app:** add cross-device screen assistance and responsive persistent tools ([#547](https://github.com/MLGBJDLW/Nexa/issues/547)) ([ea08132](https://github.com/MLGBJDLW/Nexa/commit/ea08132dd99c28aba6b58d5164abc202bb78a06a))

## [0.3.6](https://github.com/MLGBJDLW/Nexa/compare/nexa-mobile-v0.3.5...nexa-mobile-v0.3.6) (2026-11-12)

### Improvements

- Update bundled SQLite to fix a disclosed WAL memory-exposure vulnerability, preserving existing database compatibility.

- Expand archive, restore, and delete gestures across the entire conversation row, with reduced-motion support.
- Move storage usage categories and storage locations into the Data & Privacy settings tab.
- Add double-tap screenshots and a long-press action menu to the Android floating assistant while preserving drag-to-dock behavior.
- Open screen translations as editable chat drafts, preserving existing input and protecting sensitive screens.
- Prevent finished tasks from repeatedly continuing without new tool progress.
- Refresh verified provider models and endpoint-specific reasoning settings.
- Apply shared eligibility rules to automatic model selection while preserving explicitly saved models.

## [0.3.5](https://github.com/MLGBJDLW/Nexa/compare/nexa-mobile-v0.3.4...nexa-mobile-v0.3.5) (2026-11-07)

### Fixes

- Restore active and archived conversation touch swipes and full-width chat tables.
- Display uploaded image attachments after sending and reloading; offer gallery and camera selection.
- Enable inline HTML timelines and workspace image previews in standalone phone chat using the shared preview host.
- Grant the camera picker access only to app-owned Pictures and retain native file sharing from chat previews.



### Features

* **chat:** restore mobile interactions, inline media and desktop tools ([#538](https://github.com/MLGBJDLW/Nexa/issues/538)) ([b64f679](https://github.com/MLGBJDLW/Nexa/commit/b64f679337d04e869bc4721650c03de043c64365))

## [0.3.4](https://github.com/MLGBJDLW/Nexa/compare/nexa-mobile-v0.3.3...nexa-mobile-v0.3.4) (2026-11-06)

### Android and conversation controls

- Scan pairing codes through a square viewfinder in portrait or landscape. The scanner follows the device rotation preference.
- The microphone expands into a live sound-level display. System recognition can show interim words, and cloud dictation measures the audio being recorded.
- Collapse individual model settings and connection forms. Scrolling compacts settings search into a single button, and tapping it restores the query and keyboard focus.
- Steer a running phone or paired-computer turn from the composer. Follow-up fences become actionable choices and an editable draft suggestion.
- Swipe a chat left to expand its archive button. In the archive, swipe right to restore or left to confirm permanent deletion. Use the trash icon in the archive header to confirm bulk deletion.
- Reading a local reply clears its Android notification, including when opening Nexa through its launcher.

### Scheduled tasks

- Create, edit, pause and resume cron jobs from the chat drawer. Review scheduled times in their saved timezone, Web tool grants, approvals and execution history.
- Android can wake the shared executor after the app process exits. Jobs use the durable occurrence ledger and the configured model connection protected by Android Keystore. Android network and battery scheduling affect delivery time. Reopening Nexa rearms jobs after a system Force stop.


### Features

* **mobile:** improve phone conversations, scheduled tasks and model support ([#535](https://github.com/MLGBJDLW/Nexa/issues/535)) ([8b488be](https://github.com/MLGBJDLW/Nexa/commit/8b488be46355fd70c0d2d3f3269271671239569a))

## [0.3.3](https://github.com/MLGBJDLW/Nexa/compare/nexa-mobile-v0.3.2...nexa-mobile-v0.3.3) (2026-11-06)

### Voice settings

- Dictation and read-aloud each have their own model connection selector. Choose a compatible account synced from a paired computer, including Alibaba Cloud Model Studio (Bailian), and use its saved key. Existing manual speech settings are preserved.

### Conversations and Projects

- Each turn has one collapsed “Thinking and tools” section for reasoning, intermediate updates and tool calls. Expand it to see the steps in order. Final answers, user clarifications and approval requests stay visible during a run and after reopening a chat.
- Swipe a conversation or tap its archive icon to archive it with a smooth transition. Open Archived conversations to delete individual chats or clear the archive.
- Create a Project in the phone sidebar and drag chats into it, or use the move menu. New chats started in a Project join it on the first send. Project membership survives restarts and archive/restore. Removing a Project returns its chats to the ungrouped list.
- Projects are organized on the phone, with a separate list for local conversations and for each paired computer.

[Changes in #532](https://github.com/MLGBJDLW/Nexa/pull/532)

## [0.3.2](https://github.com/MLGBJDLW/Nexa/compare/nexa-mobile-v0.3.1...nexa-mobile-v0.3.2) (2026-11-06)

### Android assistant and chat

- The floating assistant gently moves at rest. Release it closer to the middle of the screen for a stronger, faster elastic return to the edge. Motion pauses while the overlay is suspended and respects disabled system animations.
- Completed replies keep the final answer in front and fold thinking and intermediate tool activity. New chats generate titles without delaying the final answer or overwriting a manual rename.
- Search the phone settings and expand only the sections you need. Cloud dictation and read-aloud reuse compatible saved connections, including keys held in Android's encrypted store.
- Swiping a conversation progressively reveals its archive action. A confirmed bulk action clears archived conversations across pages and safely skips chats restored or deleted by another client.
- Mobile now has independent version and release PR maintenance and publishes its own signed ARM64 APK without bumping desktop for phone-only changes.
- Hold the message field to dictate and release to finish without sending. The animated Plan mode switch is available beside Access, outside the attachment menu.

### Features

* **assistant:** add hold-to-dictate and mobile chat controls ([#529](https://github.com/MLGBJDLW/Nexa/issues/529)) ([0b6a3f4](https://github.com/MLGBJDLW/Nexa/commit/0b6a3f46e2b96504dea2e011f77ce92bb690da81))

## 0.3.1

- Repair narrow-screen and landscape layouts for conversation actions, search providers, storage sheets, Markdown tables, charts and HTML previews. Add left-swipe archive confirmation and original-size Mermaid viewing.
- Let dictation and read-aloud independently use phone engines or configured providers. Keep recording and playback native, protect saved credentials, support cancellation, and honor the saved speech language. Recorded-file dictation supports up to 60 seconds per recording.
- Fix first-use microphone permission recovery, empty recognition results, locale changes during voice tests and incompatible preset voices after TTS model changes.
- Expose DeepSeek Search & Fetch in a phone-oriented form, with explicit reuse of an official saved DeepSeek connection.
- Recover from temporarily unreadable accessibility UI with Home or an explicit app launch, refresh observations, and retain fresh-target checks for text, clicks and coordinate input.
- Publish only this version's mobile release notes, independently of the desktop release.

Validation includes full source CI, responsive browser regressions, Android instrumentation and a complete Android application voice smoke test against a local provider fixture. Physical Samsung/QQ behavior, microphone recognition quality and paid-provider access remain device/account acceptance checks.

## 0.3.0

- Stream canonical answer/thinking blocks during execution with durable UTF-8 offsets for reconnect replay.
- Reuse blank drafts until the first send, combine Send and Stop, and add confirmed chat archiving and deletion in the drawer.
- Expose shared web search settings including DeepSeek Search & Fetch, with keys protected by an Android-keystore-wrapped settings key.
- Keep custom system prompts intact and validate requests against actual model context capacity instead of truncating policy at a fixed size.
- Enable continuous context by default: verified history checkpoints retain recent progress without a separate summarization request or routine compaction chatter.

- Let an explicitly enabled paired computer use phone control over the existing authenticated connection; stop pending actions on cancellation or disconnect without replaying input.
- Revalidate screen pixels before coordinate taps and return fresh observations after actions or stale targets, without a fixed model-delay expiry.
- Verify Unicode text after accessibility input and support Android 13+ native editor fallback for whole-field replacement.
- Add system/on-device speech mode, persisted language preferences, service diagnostics, a dictation test and Android speech settings links.
- Dock the optional floating assistant to either edge, restore its position and respect disabled animations.
- Add categorized storage usage with paginated bulk conversation archiving and confirmed deletion.
- Preserve conversation model choices, prefer inline interactive visualizations and show local follow-up choices when model metadata is missing.
- Refresh compatible provider models and scope shared frontend checks through the mobile import graph.

Reverse phone control requires Desktop 0.19 or later and explicit per-session enablement on the phone. Process restart, Stop or pairing revocation requires enabling it again. Android permissions, an unlocked phone and a reachable paired computer remain necessary. Physical microphones, OEM background policies and authenticated provider inference are separate acceptance boundaries.

## 0.2.0

- Add first-launch computer pairing, setup deferral and an offline QR scanner.
- Explain Android restricted settings, disconnected accessibility services, notifications and vendor battery restrictions with direct recovery links.
- Add native speech dictation and answer read-aloud using Android speech engines.
- Share searchable connection/model choices, web tools, image attachments, follow-up suggestions and paged thinking history with desktop.
- Add compact Ask/Allow all controls while preserving Plan mode's read-only tool policy.
- Add Android 11+ screenshot observations, verified coordinate taps, installed-app search and an optional draggable assistant overlay with Stop.
- Keep an already-active screen visible during phone-control tasks and report protected or unavailable screens without permission retry loops.
- Scale adaptive launcher icons to a consistent safe area.
- Keep foreground task progress silent and alert only for approval requests or completed replies while the app is in the background.
- Reserve signed APK packaging for mobile releases; keep PR checks lightweight and skip verified release-only metadata builds.

System speech engines, model vision support and explicit Android permissions are required for the corresponding features. Secure screens and device policy remain protected. Physical-device/OEM behavior and paid-provider inference are separate acceptance boundaries.

## 0.1.0

- Share portable model connections and API keys with paired Desktop 0.18 or later,
  with explicit pairing grants, encrypted native storage, key rotation and
  conflict-preserving updates. Keep the active model independent on each device.

- Introduce the native Android app with separate phone and computer execution targets.
- Run phone tasks through Nexa's shared agent runtime, durable conversations, event replay, explicit approvals, cancellation and recovery checkpoints.
- Keep local provider keys and per-computer pairing tokens encrypted with Android Keystore.
- Add app-owned file creation, editing and sharing, plus optional accessibility-based observation and verified-target phone actions.
- Reuse the existing computer chat, model selection, approvals and reconnection protocol through a native HTTPS/WebSocket transport.
- Add a restrained chat layout using Nexa branding, animated history drawers and bottom sheets, swipe dismissal, Android Back handling, keyboard avoidance and reduced-motion support.
- Ship an independently versioned, signed ARM64 APK through the mobile release workflow.

Android is the first supported native platform. iOS, native HarmonyOS, local voice capture and offline model inference are not included in this release. A configured model connection is required for phone tasks. Physical-device and paid-provider acceptance are separate from compilation and emulator checks.
