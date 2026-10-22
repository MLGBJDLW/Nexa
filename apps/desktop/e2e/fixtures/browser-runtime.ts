import { readFileSync } from 'node:fs';
import { join } from 'node:path';

// Native initialization supplies the real shared SVG. DOM fixtures exercise
// the same runtime with a small inert pointer and an isolated bridge token.
export function loadBrowserRuntime(pickToken: string): string {
  return readFileSync(join(process.cwd(), 'src-tauri', 'src', 'browser', 'interaction_runtime.js'), 'utf8')
    .replace('__NEXA_PICK_TOKEN__', JSON.stringify(pickToken))
    .replace('__NEXA_POINTER_APPEARANCE__', JSON.stringify({ size: 36, hotspot: 8, accent: 'rgb(20 184 166)', reducedMotion: false, svg: '<svg viewBox="0 0 36 36"><path d="M8 8L11 28L26 21Z" fill="#14b8a6" stroke="white"/></svg>' }));
}
