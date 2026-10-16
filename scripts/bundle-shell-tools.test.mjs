import assert from 'node:assert/strict';
import { test } from 'node:test';
import { pins, platformsFor, sha256, verifyAsset } from './bundle-shell-tools.mjs';

test('Windows includes native Linux utilities for both supported WSL architectures', () => {
  assert.deepEqual(platformsFor('windows-x64'), ['windows-x64', 'linux-x64', 'linux-arm64']);
  assert.deepEqual(platformsFor('linux-x64'), ['linux-x64']);
  assert.deepEqual(platformsFor('darwin-arm64'), ['darwin-arm64']);
  assert.throws(() => platformsFor('../outside'), /Unsupported/);
  for (const tool of Object.values(pins.tools)) {
    assert.ok(tool.version && tool.repository && tool.tag);
    for (const platform of ['windows-x64', 'linux-x64', 'linux-arm64', 'darwin-arm64']) {
      assert.match(tool.assets[platform].sha256, /^[a-f0-9]{64}$/);
      assert.ok(!tool.assets[platform].file.includes('/'));
    }
  }
});

test('a changed or incomplete download cannot enter the bundle', () => {
  const bytes = Buffer.from('reviewed binary');
  const pin = { file: 'utility', sha256: sha256(bytes) };
  verifyAsset(bytes, pin);
  assert.throws(() => verifyAsset(Buffer.from('different binary'), pin), /mismatch/);
  assert.throws(() => verifyAsset(Buffer.alloc(0), { file: 'empty', sha256: sha256(Buffer.alloc(0)) }), /size/);
});
