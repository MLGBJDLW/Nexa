import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('the main WebView lets Windows file drops reach the HTML composer', () => {
  const config = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
  // Tauri's default native handler replaces WebView2's HTML5 file-drop handler.
  // Pair this launch contract with the real composer drop tests in Playwright.
  assert.equal(config.app.windows[0].dragDropEnabled, false);
});
