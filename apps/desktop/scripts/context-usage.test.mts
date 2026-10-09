import assert from 'node:assert/strict';
import { test } from 'node:test';
import { resolveContextUsage } from '../src/lib/contextUsage.ts';

test('context percent and remaining headroom use the runtime input budget', () => {
  const view = resolveContextUsage({ promptTokens: 50_000, contextWindow: 100_000, contextBreakdown: {
    totalTokens: 50_000, segments: [], measurement: 'provider',
    budget: { capacityTokens: 100_000, inputBudget: 60_000, responseReserve: 36_000, safetyReserve: 4_000, compactThreshold: 54_000, compactPercent: 90 },
  } });
  assert.equal(Math.round(view.percent), 83);
  assert.equal(view.untilCompact, 4_000);
  assert.equal(view.capacity, 100_000);
});

test('earlier compaction policies also advance the warning threshold', () => {
  const view = resolveContextUsage({ promptTokens: 40_000, contextWindow: 100_000, contextBreakdown: {
    totalTokens: 40_000, segments: [],
    budget: { capacityTokens: 100_000, inputBudget: 60_000, responseReserve: 36_000, safetyReserve: 4_000, compactThreshold: 39_000, compactPercent: 65 },
  } });
  assert.equal(view.warningPercent, 65);
  assert.ok(view.percent >= view.warningPercent);
  assert.equal(view.untilCompact, 0);
});

test('legacy and external snapshots do not invent a compaction policy', () => {
  const view = resolveContextUsage({ promptTokens: 50_000, contextWindow: 100_000 });
  assert.equal(view.percent, 50);
  assert.equal(view.compactPercent, null);
  assert.equal(view.untilCompact, null);
  assert.equal(resolveContextUsage(null).capacity, 0);
});
