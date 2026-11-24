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
const main = config.app.windows[0];
assert(main?.visible === true, 'the main window must be visible during native initialization');
assert(typeof main.backgroundColor === 'string' && main.backgroundColor !== '#ffffff', 'the native startup background must avoid a white flash');

console.log('ok - native startup visibility and background');
