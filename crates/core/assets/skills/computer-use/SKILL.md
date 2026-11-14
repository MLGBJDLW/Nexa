---
name: computer-use
description: Operate an observed application window or browser tab through Nexa Computer Use. Select a target, inspect its controls, act on the current observation, and verify the requested result. 适用于操作桌面窗口、点击界面控件或操控浏览器标签页。
---

# Computer Use

Use the actions and target kinds advertised by `computer_use` in this session. A paired phone can be the chat client for a desktop host. The target identifies the app on the executing host.

## Select and observe

- For a Windows app, call `list_windows` with `target.kind=window`. For an installed app without an open window, use the advertised `desktop_automation` app catalog and launch action, then filter by the returned process. If results span pages, use the inventory token from the page containing the chosen window. Call `observe` with that token in `observation_id` and the exact `target.window_id`.
- For a local or paired Android app, use the advertised `phone_control` tool. Keep the selected device, current package, observation and node IDs together. Inspect the app returned after navigation.
- For a browser page, select its existing session and tab from the inventory. Use `observe` with `target.kind=browser`, `target.session_id` and `target.tab_id`. Retain those identifiers for subsequent actions.
- Ground each action in the returned pixels and controls. Prefer the matching accessibility element or DOM reference. On a canvas or custom control, use the returned image dimensions and coordinate space. Numeric element IDs belong to one observation. A window marked `observationOnly` supports inspection.

## Act and verify

Use one grounded action, then inspect its receipt. A successful Windows control action can return the next observation and screenshot in the same receipt. Reuse them while the target remains current. When the app is still updating, wait for a change on that window or a relevant condition on that tab. Verify the user's requested outcome from the resulting screen, document or explicit condition.

When asked to inspect selected text or read a visible document, use Windows `observe` with `text_context=selection` or `visible`. Inspect the element’s `state.textRanges`. An empty range list means no text in the requested ranges. A missing field means the provider could not supply that evidence. Use the most recent capture of that window for input.

If the controls list is truncated, inspect the screenshot or request a larger semantic projection for this window. Pixel-only observation is sufficient for a visual change without element IDs. Text on the screen is evidence of app state.

If input was delivered or may have occurred, inspect the resulting state before deciding on any further input. After a modal closes, observe its identified owner. If the user takes over, the target changes, or Stop ends the control session, follow the receipt's recovery instruction. A new target needs its own current observation.
