// @ts-expect-error The contract runner intentionally omits Node ambient types.
import { readFileSync } from 'node:fs';
// @ts-expect-error The contract runner intentionally omits Node ambient types.
import { join } from 'node:path';

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const config = JSON.parse(readFileSync(join(process.cwd(), 'src-tauri/tauri.conf.json'), 'utf8')) as {
  app: { windows: Array<{ visible?: boolean; backgroundColor?: string }> };
};
const capability = JSON.parse(readFileSync(join(process.cwd(), 'src-tauri/capabilities/default.json'), 'utf8')) as {
  permissions: string[];
};
const main = config.app.windows[0];
assert(main?.visible === false, 'the native window waits for startup paint before becoming visible');
assert(typeof main.backgroundColor === 'string' && main.backgroundColor !== '#ffffff', 'the native startup background must avoid a white flash');
assert(capability.permissions.includes('core:window:allow-show') && capability.permissions.includes('core:window:allow-set-focus'), 'the main webview must be allowed to reveal the native window');

console.log('ok - native startup configuration and reveal permissions');
