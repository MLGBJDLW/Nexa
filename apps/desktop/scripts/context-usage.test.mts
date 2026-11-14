import assert from 'node:assert/strict';
import { test } from 'node:test';
import { modelTokensPerSecond, resolveContextUsage, retainGenerationSample } from '../src/lib/contextUsage.ts';

test('latest valid generation survives missing timing and resets on model, endpoint or conversation changes', () => {
  const measured = { outputTokens: 1400, requestDurationMs: 20000 };
  let sample = retainGenerationSample(null, 'conversation:connection:model', measured);
  sample = retainGenerationSample(sample, sample.scope, undefined);
  sample = retainGenerationSample(sample, sample.scope, { outputTokens: 1, requestDurationMs: 0 });
  assert.equal(modelTokensPerSecond(sample.metrics), 70);
  const external = { outputTokens: 100, requestDurationMs: 2000, externalTurn: true };
  sample = retainGenerationSample(sample, sample.scope, external);
  assert.equal(sample.metrics?.externalTurn, true);
  for (const scope of ['conversation:connection:other-model', 'conversation:other-connection:model', 'other-conversation:connection:model']) {
    const switched = retainGenerationSample(sample, scope, external);
    assert.equal(switched.metrics, undefined);
    assert.equal(retainGenerationSample(switched, scope, external).metrics, undefined);
    const next = retainGenerationSample(switched, scope, { outputTokens: 240, requestDurationMs: 2000 });
    assert.equal(modelTokensPerSecond(next.metrics), 120);
  }
});

test('generation speed uses one model request and rejects unavailable or invalid timing', () => {
  assert.equal(modelTokensPerSecond({outputTokens: 120, requestDurationMs: 2000}), 60);
  assert.equal(modelTokensPerSecond({outputTokens: 0, requestDurationMs: 2000}), 0);
  for (const metrics of [undefined, {outputTokens: 120, requestDurationMs: 0}, {outputTokens: -1, requestDurationMs: 10}, {outputTokens: Infinity, requestDurationMs: 10}]) {
    assert.equal(modelTokensPerSecond(metrics), null);
  }
});

test('runtime-reported capacity survives a different next-turn model selection', () => {
  const view = resolveContextUsage({ promptTokens: 50_000, contextWindow: 1_000_000, contextBreakdown: {
    totalTokens: 50_000, segments: [], contextWindow: 200_000,
    runtimeProvider: 'github_copilot', runtimeModel: 'running-model',
  } });
  assert.equal(view.capacity, 200_000);
  assert.equal(view.percent, 25);
});

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
