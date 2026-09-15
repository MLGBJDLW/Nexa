// @ts-expect-error The contract runner intentionally omits Node ambient types.
import { readFileSync } from 'node:fs';
// @ts-expect-error The contract runner intentionally omits Node ambient types.
import { join } from 'node:path';

type TestFn = () => void;
const tests: Array<{ name: string; fn: TestFn }> = [];

function test(name: string, fn: TestFn): void {
  tests.push({ name, fn });
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function source(relativePath: string): string {
  return readFileSync(join(process.cwd(), relativePath), 'utf8');
}

test('remote browser webviews do not inherit the main application capability', () => {
  const capability = JSON.parse(source('src-tauri/capabilities/default.json')) as {
    windows?: string[];
    webviews?: string[];
  };
  assert(!capability.windows?.includes('main'), 'window-wide capability would grant every child webview IPC access');
  assert(capability.webviews?.length === 1, 'only one privileged webview should be declared');
  assert(capability.webviews?.[0] === 'main', 'only the main application webview should be privileged');
});

test('Browser Workspace uses a native top-level child webview instead of an iframe', () => {
  const host = source('src-tauri/src/browser/webview_host.rs');
  const dock = source('src/features/browser/BrowserDock.tsx');
  const tauriConfig = source('src-tauri/tauri.conf.json');
  assert(host.includes('WebviewBuilder::new'), 'desktop host must create a native child WebView');
  assert(host.includes('WebviewUrl::External'), 'remote pages must load as top-level WebView documents');
  assert(host.includes('.data_directory('), 'browser profiles need an isolated data directory');
  assert(host.includes('.data_store_identifier('), 'macOS browser profiles need an isolated website data store');
  assert(tauriConfig.includes('"minimumSystemVersion": "14.0"'), 'macOS must support isolated data stores and policy proxies');
  assert(!host.includes('.enable_clipboard_access()'), 'remote pages must not receive unconditional JavaScript clipboard access');
  assert(host.includes('.initialization_script_for_all_frames('), 'trusted-input takeover must cover embedded frames');
  assert(host.includes('.proxy_url('), 'all browser subresources must pass through the network policy proxy');
  assert(host.includes('--proxy-bypass-list=<-loopback>'), 'Windows must not bypass the policy proxy for loopback targets');
  assert(!host.includes('USER_TAKEOVER_TITLE_SIGNAL'), 'page-controlled titles must never authenticate user takeover');
  assert(!dock.includes('<iframe'), 'BrowserDock must not regress to iframe embedding');
});


for (const { name, fn } of tests) {
  try {
    fn();
    console.log(`ok - ${name}`);
  } catch (error) {
    console.error(`not ok - ${name}`);
    throw error;
  }
}
