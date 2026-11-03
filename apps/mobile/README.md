# Nexa Mobile

Native Android application, independently versioned from Nexa Desktop. The first
release is `0.1.0`; its tag prefix is `nexa-mobile-v`.

See the [mobile application guide](../../docs/guides/MOBILE_APP.md) for setup,
execution targets, permissions, acceptance boundaries and release instructions.
The existing browser companion remains available through
[Phone access](../../docs/guides/remote-access.md).

Pair with Desktop 0.18 or later to share saved cloud model connections and API
keys in both directions. Phone settings can also add independent connections.
Each device keeps its own selected model, tools, history and workspace.

```sh
npm ci --prefix apps/desktop
npm ci --prefix apps/mobile
npm run build --prefix apps/mobile
npm run tauri --prefix apps/mobile -- android build --apk --target aarch64
```

Use JDK 21, Android SDK 36, NDK `27.2.12479018`, and the Rust Android targets.
Set `ANDROID_HOME` and `NDK_HOME`. Windows native Android builds require Developer
Mode for Tauri's library symlinks; release CI runs on Linux. The mobile Cargo
workspace and lockfile keep desktop-only feature dependencies out of the APK.
