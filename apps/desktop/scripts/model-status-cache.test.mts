import assert from 'node:assert/strict';
import test from 'node:test';
import { getModelStatus, invalidate, invalidateAll } from '../src/lib/modelStatusCache.ts';

test('an invalidated in-flight probe cannot restore a deleted model or evict its newer probe', async () => {
  invalidateAll();
  let finishOld!: (value: boolean) => void;
  let finishNew!: (value: boolean) => void;
  const old = getModelStatus('embed', 'model', () => new Promise<boolean>(resolve => { finishOld = resolve; }));
  await Promise.resolve();
  invalidate('embed');
  const fresh = getModelStatus('embed', 'model', () => new Promise<boolean>(resolve => { finishNew = resolve; }));
  finishOld(true);
  await old;
  await Promise.resolve();
  const coalesced = getModelStatus('embed', 'model', () => { throw new Error('new probe was evicted'); });
  finishNew(false);
  assert.equal(await fresh, false);
  assert.equal(await coalesced, false);
  assert.equal(await getModelStatus('embed', 'model', () => { throw new Error('expected fresh cache'); }), false);
});
