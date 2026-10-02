import assert from 'node:assert/strict';
import test from 'node:test';
import { compareTaskEval } from './compare-task-eval.mjs';

const baseline = { schemaVersion: 1, mode: 'scripted', scoreKind: 'scripted_runtime_contract', sourceSha: 'before', sourceDirty: false, corpusDigest: 'corpus', model: 'model', providerEndpointId: 'route', nodeVersion: 'v24', repetitions: 1, maxTokensPerTask: 1000, timeoutSeconds: 10, prices: null, passed: 1, total: 1, cases: [{ taskId: 'fix', repetition: 1, passed: true, elapsedMs: 10, providerInvocations: [{}], reportedUsageCost: null }] };
test('records lost task success without disguising it as a faster run', () => {
  const candidate = structuredClone(baseline);
  candidate.sourceSha = 'after'; candidate.passed = 0; candidate.cases[0].passed = false; candidate.cases[0].elapsedMs = 2;
  const result = compareTaskEval(baseline, candidate);
  assert.deepEqual(result.regressions, ['fix/1']);
  assert.equal(result.cases[0].elapsedDeltaMs, -8);
  assert.equal(result.cases[0].reportedUsageCostDelta, null);
});
test('does not compare fixtures, model choices, budgets, or dirty source as equivalent runs', () => {
  for (const [field, value] of [['mode','live'], ['model','other'], ['corpusDigest','changed'], ['maxTokensPerTask',2000], ['sourceDirty',true]]) {
    const candidate = { ...baseline, [field]: value };
    assert.throws(() => compareTaskEval(baseline, candidate));
  }
});
test('rejects duplicated or missing cases', () => {
  assert.throws(() => compareTaskEval(baseline, { ...baseline, cases: [] }));
});
