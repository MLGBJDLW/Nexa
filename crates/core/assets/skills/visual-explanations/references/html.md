# Inline HTML

Use a `nexa-html` fence for a self-contained document with inline CSS and plain
JavaScript. Browser-native SVG/canvas and local form controls are supported.
Keep the document within 512 KiB. Raw JSX, TSX, TypeScript, npm imports, CDN
libraries and relative assets need a compiled file preview instead.

1. Include an accessible title, input labels, default values, clear units and a
   static explanation that remains useful before scripts run.
2. Use responsive CSS (`max-width:100%`, wrapping grids) and system fonts. Honor
   `prefers-reduced-motion`; avoid continuous animation unless it explains the
   phenomenon. Keep the default content near 360px tall and allow scrolling.
3. Use `textContent` for user input. Keep calculations local, validate input
   ranges and show errors next to the relevant field. Label illustrative data.
4. Close every script/style tag and the fence. During streaming scripts are
   inert. Once complete, the user chooses Run; Stop, edits, and closing the
   preview destroy its session. State is ephemeral and resets on reload.

The host supplies nested opaque-origin sandbox frames. Network requests,
external resources, navigation outside the preview origin, popups, downloads,
forms submission, storage, parent DOM and Nexa/OS APIs are unavailable. There
is no `window.openai`, MCP Apps bridge, `invoke`, or tool-call action. Use actual
Nexa tools for durable side effects.

Example:

```nexa-html
<!doctype html><html lang="en"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>body{font:15px system-ui;margin:24px;color:#263347}input{width:100%}output{font-size:24px} @media(prefers-reduced-motion:reduce){*{animation:none!important}}</style>
<h2>Distance at a constant speed</h2>
<p>Illustration: distance = speed × 2 hours.</p>
<label for="speed">Speed (km/h)</label>
<input id="speed" type="range" min="0" max="120" value="40">
<p>Distance: <output id="distance" for="speed">80 km</output></p>
<script>document.getElementById('speed').addEventListener('input',event=>{document.getElementById('distance').textContent=(Number(event.target.value)*2)+' km';});</script>
</html>
```
