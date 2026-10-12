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

Useful semantic color slots include `surface0`, `surface1`, `textPrimary`, `textSecondary`, `thinkingText`, `replyText`, `accent`, `border`, `success`, `warning`, and `danger`. Optional objects use the following fields:

| Object | Supported fields |
| --- | --- |
| `typography` | `fontFamily`, `monoFontFamily` (local font-family names); `baseSize` 12–20; `lineHeight` 1.2–2; `letterSpacing` -0.04–0.12 |
| `motion` | `durationScale` 0–2; `cursorStyle`: `fluid`, `precise`, or `minimal` |
| `effects` | `surfaceOpacity` 0.35–1; `glassBlur` 0–48; `shadowIntensity` 0–2; `radiusScale` 0.5–2; `densityScale` 0.8–1.25 |
| `brand` | `logoVariant`: `auto`, `monochrome`, or `accent`; `logoForeground`, `logoMuted` (colors); `logoOpacity` 0.4–1 |
| `content` | `tagline` up to 160 characters, `statusText` up to 80, `quote` up to 240; plain decorative text |
| `components` | `rail`, `header`, `card`, or `browser`; each accepts `background` (color/gradient), `borderColor`, and `boxShadow` |

For example, `"typography": {"fontFamily": "Inter, sans-serif", "baseSize": 16}` changes interface text. Numbers outside the ranges are clamped. Resolve validation errors and check that the normalized draft retains every intended change before applying; unknown object fields are not customization slots.

Backgrounds can use a color or gradient `value`; image backgrounds require an existing managed `assetId`. For a new local image, use Settings → Themes → Import background, then read the installed theme to reuse its managed id. Do not fabricate image ids or put file paths or remote URLs into theme data. Theme resources cannot execute scripts or arbitrary CSS selectors. Use existing tool operations instead of editing app files, the registry database, or extension directories directly.
