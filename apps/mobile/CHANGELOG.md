# Changelog

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
