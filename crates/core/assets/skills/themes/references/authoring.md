# Authoring a theme

A minimal `appearance` draft:

```json
{
  "action": "draft",
  "plugin": {
    "manifestVersion": 2,
    "kind": "theme-resource",
    "id": "ocean",
    "name": "Ocean",
    "theme": {
      "baseTheme": "dark",
      "mode": "dark",
      "colors": { "accent": "#68b8c6" },
      "background": { "kind": "none" }
    }
  }
}
```

Choose a base from `dark`, `light`, `midnight`, `aurora`, `bloom`, or `dream`. Mode is `dark` or `light`. Set only the overrides needed for the requested look, preserving the other fields when editing an installed theme. Keep text readable against panels and retain distinct status colors.

The tool schema describes optional typography, motion, brand, content, effects, and component fields. Useful semantic color slots include `surface0`, `surface1`, `textPrimary`, `textSecondary`, `thinkingText`, `replyText`, `accent`, `border`, `success`, `warning`, and `danger`. Resolve draft validation errors before applying.

Backgrounds can use a color or gradient `value`; image backgrounds require an existing managed `assetId`. For a new local image, use Settings → Themes → Import background, then read the installed theme to reuse its managed id. Do not fabricate image ids or put file paths or remote URLs into theme data. Theme resources cannot execute scripts or arbitrary CSS selectors. Use existing tool operations instead of editing app files, the registry database, or extension directories directly.
