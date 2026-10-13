# Authoring a theme

A minimal `appearance` change:

```json
{
  "action": "apply",
  "plugin": {
    "name": "Ocean",
    "theme": {
      "colors": { "accent": "#68b8c6" }
    }
  }
}
```

The name can contain Unicode and spaces. Nexa derives an internal id for a new name; applying an existing unique name reuses its id. Explicit old ids remain supported. `manifestVersion`, `kind`, `id`, `baseTheme`, `mode`, and the background are optional. A new minimal theme defaults to dark. Imported JSON files may use friendly filenames such as `Ocean.json` or `海风.json`.

Choose an optional base from `dark`, `light`, `midnight`, `aurora`, `bloom`, or `dream`. Mode is `dark` or `light`, inferred from the base when omitted. Set only the overrides needed for a new look; preserve the other fields when editing an installed theme. Keep text readable against panels and retain distinct status colors.

Useful semantic color slots include `surface0`, `surface1`, `textPrimary`, `textSecondary`, `thinkingText`, `replyText`, `accent`, `border`, `success`, `warning`, and `danger`. Optional objects use the following fields:

| Object | Supported fields |
| --- | --- |
| `typography` | `fontFamily`, `monoFontFamily` (local font-family names); `baseSize` 12–20; `lineHeight` 1.2–2; `letterSpacing` -0.04–0.12 |
| `motion` | `durationScale` 0–2; `cursorStyle`: `fluid`, `precise`, or `minimal` |
| `effects` | `surfaceOpacity` 0.35–1; `glassBlur` 0–48; `shadowIntensity` 0–2; `radiusScale` 0.5–2; `densityScale` 0.8–1.25 |
| `brand` | `logoVariant`: `auto`, `monochrome`, or `accent`; `logoForeground`, `logoMuted` (colors); `logoOpacity` 0.4–1 |
| `content` | `tagline` up to 160 characters, `statusText` up to 80, `quote` up to 240; plain decorative text |
| `components` | `rail`, `header`, `card`, or `browser`; each accepts `background` (color/gradient), `borderColor`, and `boxShadow` |

For example, `"typography": {"fontFamily": "Inter, sans-serif", "baseSize": 16}` changes interface text. Numbers outside the ranges are clamped. Both `draft` and `apply` validate the declaration; resolve validation errors before retrying. Unknown object fields are not customization slots.

Backgrounds can use a color or gradient `value`; image backgrounds require an existing managed `assetId`. For a new local image, use Settings → Themes → Import background, then read the installed theme to reuse its managed id. Do not fabricate image ids or put file paths or remote URLs into theme data. Theme resources cannot execute scripts or arbitrary CSS selectors. Use the appearance tool for live changes; edit portable declaration files when the user requests file-level work. The registry database and application code are not theme-authoring surfaces.
