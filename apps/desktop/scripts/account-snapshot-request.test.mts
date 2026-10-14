import assert from 'node:assert/strict';
import { test } from 'node:test';
import { accountSnapshotRequest, invalidateAccountSnapshot } from '../src/lib/accountSnapshotRequest.ts';

test('account transitions reject old pending reads and allow a fresh account immediately', async () => {
  let complete!: (value: string) => void;
  const old = accountSnapshotRequest('transition', () => new Promise<string>(resolve => { complete = resolve; }));
  await Promise.resolve();
  const rejected = assert.rejects(old, /Account changed/);
  invalidateAccountSnapshot('transition');
  const fresh = accountSnapshotRequest('transition', async () => 'new account');
  complete('old account');
  await rejected;
  assert.equal(await fresh, 'new account');
});

test('a stalled account read leaves loading, permits retry, and ignores its late result', async () => {
  let complete!: (value: string) => void;
  const stalled = new Promise<string>(resolve => { complete = resolve; });
  const request = accountSnapshotRequest('codex', () => stalled, 5);
  assert.equal(accountSnapshotRequest('codex', () => Promise.resolve('duplicate')), request);
  await assert.rejects(request, /timed out/);
  assert.equal(await accountSnapshotRequest('codex', async () => 'fresh'), 'fresh');
  complete('stale');
  await assert.rejects(request, /timed out/);
});

test('account requests settle independently and a rejection does not poison retries', async () => {
  await assert.rejects(accountSnapshotRequest('copilot', async () => { throw new Error('offline'); }), /offline/);
  assert.deepEqual(await Promise.all([
    accountSnapshotRequest('copilot', async () => 0),
    accountSnapshotRequest('hermes:workspace-b', async () => 42),
  ]), [0, 42]);
});
