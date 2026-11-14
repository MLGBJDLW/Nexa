# Nexa Mobile

Native Android application, independently versioned from Nexa Desktop. Its release tag prefix is `nexa-mobile-v`.

See the [mobile application guide](../../docs/guides/MOBILE_APP.md) for setup,
execution targets, permissions, acceptance boundaries and release instructions.
The existing browser companion remains available through
[Phone access](../../docs/guides/remote-access.md).

Pair with Desktop 0.18 or later to share saved cloud model connections and API
keys in both directions. Phone settings can also add independent connections.
Each device keeps its own selected model, tools, history and workspace.

Chat history keeps older pages and expanded replies when it refreshes, while
applying updated attachment and steering-message metadata. Earlier turns remain
stable as new output streams, so long conversations avoid repeatedly processing
the same saved replies. These behaviors are shared with the browser companion.

Interactive reply previews stay visible when their activity details are
collapsed. With an official DeepSeek Flash connection, Auto web search streams
hosted searches and URL reads in the phone chat. Nexa Router uses the search
providers configured in phone settings.

On screens with many Android accessibility nodes, model context keeps complete
nodes and their original IDs. Focused and actionable controls are shown first.
The observation identifies the current package and reports how many nodes were
omitted from model context.

Desktop 0.19 or later can also operate the phone after **Allow this computer to
control this phone** is enabled in phone settings. The existing foreground-task
notification provides Stop. Reconnection retains an enabled session but never
replays uncertain input; restarting the phone app requires enabling control again.

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
