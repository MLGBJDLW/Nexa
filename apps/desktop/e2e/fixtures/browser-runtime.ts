import { readFileSync } from 'node:fs';
import { join } from 'node:path';

// Match the native theme payload and isolate the fixture's bridge token.
export function loadBrowserRuntime(pickToken: string): string {
  return readFileSync(join(process.cwd(), 'src-tauri', 'src', 'browser', 'interaction_runtime.js'), 'utf8')
    .replace('__NEXA_PICK_TOKEN__', JSON.stringify(pickToken))
    .replace('__NEXA_POINTER_APPEARANCE__', JSON.stringify({ accent: 'rgb(20 184 166)', reducedMotion: false }));
}
