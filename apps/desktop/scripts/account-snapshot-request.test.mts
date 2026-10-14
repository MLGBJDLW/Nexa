import assert from 'node:assert/strict';
import { test } from 'node:test';
import { accountSnapshotRequest } from '../src/lib/accountSnapshotRequest.ts';

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
