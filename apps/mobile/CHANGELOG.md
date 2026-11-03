# Changelog

## 0.1.0

- Introduce the native Android app with separate phone and computer execution targets.
- Run phone tasks through Nexa's shared agent runtime, durable conversations, event replay, explicit approvals, cancellation and recovery checkpoints.
- Keep local provider keys and per-computer pairing tokens encrypted with Android Keystore.
- Add app-owned file creation, editing and sharing, plus optional accessibility-based observation and verified-target phone actions.
- Reuse the existing computer chat, model selection, approvals and reconnection protocol through a native HTTPS/WebSocket transport.
- Add a restrained chat layout using Nexa branding, animated history drawers and bottom sheets, swipe dismissal, Android Back handling, keyboard avoidance and reduced-motion support.
- Ship an independently versioned, signed ARM64 APK through the mobile release workflow.

Android is the first supported native platform. iOS, native HarmonyOS, local voice capture and offline model inference are not included in this release. A configured model connection is required for phone tasks. Physical-device and paid-provider acceptance are separate from compilation and emulator checks.
