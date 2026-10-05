import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { inventory, selectPrunable, within } from './dev-cache.mjs';

test('quota evicts the oldest cache groups and keeps recent groups within budget', () => {
  const now = Date.now();
  const entries = [30, 4, 1].map((age, i) => ({ path: `${i}`, bytes: 10, modified: now - age * 86400000 }));
  assert.deepEqual(selectPrunable(entries, 15, 14, now).map(e => e.path), ['0', '1']);
  assert.equal(within('/repo', '/repo-other/cache'), false);
  assert.equal(within('/repo', '/repo'), false);
  assert.equal(within('/repo', '/repo/target/debug'), true);
});

test('inventory only includes known development caches and rejects junction escapes', async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'nexa-cache-'));
  const outside = await fs.mkdtemp(path.join(os.tmpdir(), 'nexa-outside-'));
  try {
    for (const dir of ['target/debug', 'target/release', 'target/copilot-cli-cache', 'apps/desktop/test-results-old', 'apps/desktop/user-data']) {
      await fs.mkdir(path.join(root, dir), { recursive: true });
      await fs.writeFile(path.join(root, dir, 'data'), 'abc');
    }
    const entries = await inventory(root);
    assert.equal(entries.length, 2);
    await fs.mkdir(path.join(root, 'apps/desktop/node_modules'), { recursive: true });
    await fs.symlink(outside, path.join(root, 'apps/desktop/node_modules/.vite'), process.platform === 'win32' ? 'junction' : 'dir');
    await assert.rejects(inventory(root), /linked cache/);
  } finally {
    assert(within(os.tmpdir(), await fs.realpath(root)));
    assert(within(os.tmpdir(), await fs.realpath(outside)));
    await fs.rm(root, { recursive: true, force: true });
    await fs.rm(outside, { recursive: true, force: true });
  }
});

test('scratch Cargo targets share the quota without including source or release files', async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'nexa-scratch-cache-'));
  try {
    for (const dir of ['target', 'target/audit', '.artifacts/probe-target', '.codex_tmp/probe/target']) {
      await fs.mkdir(path.join(root, dir, 'debug/deps'), { recursive: true });
      await fs.mkdir(path.join(root, dir, 'release'), { recursive: true });
      await fs.writeFile(path.join(root, dir, '.rustc_info.json'), '{}');
      await fs.writeFile(path.join(root, dir, 'debug/deps/binary'), 'rebuildable');
    }
    await fs.mkdir(path.join(root, '.artifacts/source/debug'), { recursive: true });
    await fs.writeFile(path.join(root, '.artifacts/source/debug/source.rs'), 'source');
    const entries = await inventory(root);
    assert.deepEqual(entries.map(entry => path.relative(root, entry.path)).sort(), [
      '.artifacts/probe-target/debug', '.codex_tmp/probe/target/debug', 'target/audit/debug', 'target/debug',
    ].map(value => path.normalize(value)).sort());
    assert.equal(new Set(entries.map(entry => entry.path)).size, entries.length);
  } finally {
    assert(within(os.tmpdir(), await fs.realpath(root)));
    await fs.rm(root, { recursive: true, force: true });
  }
});

test('scratch discovery never traverses a linked directory', async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'nexa-scratch-link-'));
  const outside = await fs.mkdtemp(path.join(os.tmpdir(), 'nexa-outside-target-'));
  try {
    await fs.mkdir(path.join(outside, 'debug'));
    await fs.writeFile(path.join(outside, '.rustc_info.json'), '{}');
    await fs.symlink(outside, path.join(root, '.artifacts'), process.platform === 'win32' ? 'junction' : 'dir');
    assert.deepEqual(await inventory(root), []);
  } finally {
    assert(within(os.tmpdir(), await fs.realpath(root)));
    assert(within(os.tmpdir(), await fs.realpath(outside)));
    await fs.rm(root, { recursive: true, force: true });
    await fs.rm(outside, { recursive: true, force: true });
  }
});
