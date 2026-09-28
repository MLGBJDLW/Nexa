import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

function harness(invoke) {
  let now = 1_000_000;
  const source = readFileSync(new URL('../src/lib/subscriptionModelCatalog.ts', import.meta.url), 'utf8');
  const code = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } }).outputText;
  const module = { exports: {} };
  vm.runInNewContext(code, { module, exports: module.exports, require: () => ({ invoke }), setTimeout, clearTimeout, Date: { now: () => now } });
  return { ...module.exports, advance: milliseconds => { now += milliseconds; } };
}
const models = [{ id: 'native', name: 'Native', reasoningEfforts: ['low', 'max'] }];

test('concurrent consumers, reopen and reasoning edits share one catalog request', async () => {
  let calls = 0, resolve;
  const cache = harness(() => { calls++; return new Promise(done => { resolve = done; }); });
  const first = cache.loadSubscriptionModels('github_copilot');
  const second = cache.loadSubscriptionModels('github_copilot');
  assert.equal(first, second);
  assert.equal(cache.getSubscriptionCatalogs().github_copilot.loading, true);
  resolve(models); await first;
  await cache.loadSubscriptionModels('github_copilot');
  assert.equal(calls, 1);
  cache.advance(5 * 60_000 + 1);
  const refresh = cache.loadSubscriptionModels('github_copilot');
  assert.equal(cache.getSubscriptionCatalogs().github_copilot.models, models);
  resolve([]); await refresh;
  assert.equal(cache.getSubscriptionCatalogs().github_copilot.models.length, 0);
  assert.equal(calls, 2);
});

test('failed refresh preserves usable data and throttles automatic retries', async () => {
  let calls = 0;
  const cache = harness(async () => { if (calls++) throw new Error('offline'); return models; });
  await cache.loadSubscriptionModels('openai_codex');
  await assert.rejects(cache.loadSubscriptionModels('openai_codex', true), /offline/);
  assert.equal(cache.getSubscriptionCatalogs().openai_codex.models, models);
  await assert.rejects(cache.loadSubscriptionModels('openai_codex'), /offline/);
  assert.equal(calls, 2);
});

test('account change rejects late results and never restores the old account catalog', async () => {
  const resolvers = [];
  const cache = harness(() => new Promise(resolve => resolvers.push(resolve)));
  cache.reconcileSubscriptionAccount('github_copilot', 'old');
  const old = cache.loadSubscriptionModels('github_copilot');
  const rejected = assert.rejects(old, /account changed/);
  cache.reconcileSubscriptionAccount('github_copilot', 'new');
  const current = cache.loadSubscriptionModels('github_copilot');
  resolvers[1]([]); await current;
  resolvers[0](models); await rejected;
  assert.equal(cache.getSubscriptionCatalogs().github_copilot.models.length, 0);
});

test('ACP profiles keep independent catalogs and invalidate only the edited launch', async () => {
  const requests = [];
  const cache = harness(async (_command, args) => { requests.push(args); return [{ id: args.agentConfigId, name: args.agentConfigId, reasoningEfforts: [] }]; });
  await cache.loadSubscriptionModels('hermes', false, 'project-a');
  await cache.loadSubscriptionModels('hermes', false, 'project-b');
  const a = cache.runtimeCatalogKey('hermes', 'project-a');
  const b = cache.runtimeCatalogKey('hermes', 'project-b');
  assert.equal(cache.getSubscriptionCatalogs()[a].models[0].id, 'project-a');
  assert.equal(cache.getSubscriptionCatalogs()[b].models[0].id, 'project-b');
  cache.invalidateSubscriptionModels(a);
  await cache.loadSubscriptionModels('hermes', false, 'project-b');
  assert.equal(requests.length, 2);
  assert.equal(cache.getSubscriptionCatalogs()[b].models[0].id, 'project-b');
});
