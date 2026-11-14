import assert from 'node:assert/strict';
import {
  compareEndpointModels,
  compareRequiredModelIds,
  driftDetected,
  normalizeCatalogModel,
  inspectCatalogPage,
  inventoryCandidates,
  summarizeDiscovery,
} from './model-catalog-audit-lib.mjs';

const endpoint = {
  endpointId: 'text:tenant-a',
  models: [{
    id: 'model-a',
    aliases: ['model-a-latest'],
    lifecycle: 'active',
    regions: ['us-east'],
    capabilities: { toolCalling: true },
  }],
};

const changed = compareEndpointModels(endpoint, [{
  id: 'model-a',
  status: 'deprecated',
  regions: ['eu-west'],
  capabilities: { toolCalling: false },
}]);
assert.deepEqual(changed.capabilityChanged, [{ id: 'model-a', fields: ['toolCalling'] }]);
assert.deepEqual(changed.lifecycleChanged, [{ id: 'model-a', curated: 'active', discovered: 'deprecated' }]);
assert.equal(changed.regionChanged.length, 1);
assert.equal(driftDetected(changed), true);

const isolated = compareEndpointModels(endpoint, [{ id: 'tenant-b-only-model' }]);
const announced = compareEndpointModels(endpoint, [{ id: 'model-a', expiration_date: '2026-11-18' }]);
assert.deepEqual(announced.lifecycleChanged, [{ id:'model-a', curated:'active', discovered:'deprecated' }]);
assert.deepEqual(announced.expirationChanged, [{ id:'model-a', curated:null, discovered:'2026-11-18' }]);
assert.deepEqual(isolated.newIds, ['tenant-b-only-model']);
assert.deepEqual(isolated.missingIds, ['model-a']);
assert.equal(isolated.capabilityChanged.length, 0);

const clean = compareEndpointModels(endpoint, [{
  id: 'model-a',
  regions: ['us-east'],
  capabilities: { toolCalling: true },
}]);
assert.equal(driftDetected(clean), false);

const aliasOnly = compareEndpointModels(endpoint, [{
  id: 'MODEL-A-LATEST',
  regions: ['us-east'],
  capabilities: { toolCalling: true },
}]);
assert.deepEqual(aliasOnly.newIds, []);
assert.deepEqual(aliasOnly.missingIds, []);
assert.equal(driftDetected(aliasOnly), false);

const requiredPublicIds = ['z-ai/glm-5.3', 'z-ai/glm-5.3-flash'];
const resolvedAliases = compareEndpointModels({ models: [
  { id:'jev-latest', discoveryId:'~typesafe/jev-latest', capabilities:{} },
  { id:'~typesafe/jev-latest', capabilities:{} },
] }, [{ id:'~typesafe/jev-latest' }]);
assert.deepEqual(resolvedAliases.missingIds, []);
assert.deepEqual(resolvedAliases.newIds, []);
const publicExact = compareRequiredModelIds(requiredPublicIds, [
  { id: 'unrelated/provider-model' },
  { id: 'Z-AI/GLM-5.3' },
  { id: 'z-ai/glm-5.3-flash' },
]);
assert.deepEqual(publicExact.newIds, []);
assert.deepEqual(publicExact.missingIds, []);
assert.equal(driftDetected(publicExact), false);
const publicMissing = compareRequiredModelIds(requiredPublicIds, [{ id: 'z-ai/glm-5.3' }]);
assert.deepEqual(publicMissing.missingIds, ['z-ai/glm-5.3-flash']);
assert.equal(driftDetected(publicMissing), true);
process.stdout.write('model catalog audit fixtures passed\n');

assert.deepEqual(normalizeCatalogModel('video', { modelId: 'MiniMax-H3', displayName: 'MiniMax H3', releaseStatus: 'ga', selectable: true }), { modelId: 'MiniMax-H3', displayName: 'MiniMax H3', releaseStatus: 'ga', selectable: true, id: 'MiniMax-H3', name: 'MiniMax H3', status: 'active' });
assert.equal(normalizeCatalogModel('video', { modelId: 'gated', displayName: 'Gated', releaseStatus: 'unverified' }).status, 'gated');

// A green required-ID contract must not hide a new, unreviewed model.
const routerEndpoint = { endpointId: 'text:openrouter', models: [{ id: 'known', capabilities: {} }] };
const routerRows = [
  { id: 'known', architecture: { output_modalities: ['text'] }, endpoints: ['/api/v1/chat/completions'] },
  { id: 'new:free', architecture: { output_modalities: ['text'] }, endpoints: ['/api/v1/chat/completions'] },
  { id: 'new:batch', architecture: { output_modalities: ['text'] }, endpoints: ['/api/v1/batches'] },
  { id: 'voice', architecture: { output_modalities: ['speech'] } },
  { id: 'text-extraction', architecture: { output_modalities: ['text'] }, endpoints: ['/api/alpha/extractions'] },
];
const inventory = inventoryCandidates(routerEndpoint, routerRows);
assert.deepEqual(inventory.newIds, ['new:free']);
assert.deepEqual(inventory.excluded.map(row => row.reason), ['batch_only', 'other_modality', 'no_chat_endpoint']);
assert.deepEqual(inventory.modalityCounts, { text: 4, speech: 1 });
const contract = compareRequiredModelIds(['known'], routerRows);
assert.equal(driftDetected(contract), false);
assert.equal(summarizeDiscovery([{ status: 'ok', endpointId: 'text:openrouter', inventory,
  pagination: { completeness: 'complete' } }]).hasInventoryCandidates, true);
assert.deepEqual(inventoryCandidates({ endpointId: 'text:siliconflow', models: [] }, [{ id: 'vendor/new' }]).newIds, ['vendor/new']);
assert.deepEqual(inventoryCandidates({ endpointId: 'text:alibaba-model-studio', models: [] }, [{ id: 'vendor/new', output_modalities: ['text'] }]).newIds, ['vendor/new']);
assert.throws(() => inspectCatalogPage({ error: 'bad token' }), /invalid_catalog_payload/);
assert.throws(() => inspectCatalogPage({ data: [{}] }), /invalid_catalog_payload/);
assert.throws(() => inspectCatalogPage({ data: [{ id: 'x' }, { id: 'X' }] }), /duplicate_catalog_ids/);
assert.throws(() => inspectCatalogPage({ data: [{ id: 'x' }], total_count: 0 }), /invalid_catalog_total/);
assert.equal(inspectCatalogPage({ data: [], total_count: 0, links: { next: null } }).pagination.completeness, 'complete');
assert.equal(inspectCatalogPage({ models: [] }).pagination.completeness, 'unknown');
assert.equal(inspectCatalogPage({ data: [{ id: 'x' }], total_count: 2 }).pagination.completeness, 'partial');
assert.equal(inspectCatalogPage({ data: [], links: { next: 'https://untrusted.invalid' } }).pagination.completeness, 'partial');
assert.equal(summarizeDiscovery([{ status: 'ok', endpointId: 'partial', pagination: { completeness: 'partial' } }]).requiresInventoryReview, true);

// Run the real CLI with an injected HTTP boundary: no provider calls or keys.
const { mkdtemp, writeFile, readFile, rm } = await import('node:fs/promises');
const { tmpdir } = await import('node:os');
const { join } = await import('node:path');
const { execFileSync } = await import('node:child_process');
const { fileURLToPath } = await import('node:url');
const testDir = await mkdtemp(join(tmpdir(), 'nexa-inventory-'));
try {
  const mock = join(testDir, 'fetch.mjs');
  await writeFile(mock, `globalThis.fetch = async (url) => {
    const u = new URL(url);
    if (u.hostname === 'api.siliconflow.cn') {
      if (u.searchParams.get('sub_type') !== 'chat') throw new Error('missing chat filter');
      return { ok: true, json: async () => ({data:[{id:'fixture/new-chat'}]}) };
    }
    if (u.hostname === 'openrouter.ai' && u.searchParams.get('output_modalities') === 'all')
      return { ok: true, json: async () => ({data:${JSON.stringify(routerRows)}, total_count:6, links:{next:'https://untrusted.invalid/next'}}) };
    return { ok: true, json: async () => ({error:'not a catalog'}) };
  };`);
  const out = join(testDir, 'report');
  const stdout = execFileSync(process.execPath, ['--import', mock, fileURLToPath(new URL('./model-catalog-audit.mjs', import.meta.url)), '--live-public', '--output', out], {
    encoding: 'utf8', env: { PATH: process.env.PATH, NEXA_SILICONFLOW_CN_CATALOG_API_KEY: 'fixture-not-a-real-key' },
  });
  const report = JSON.parse(await readFile(join(out, 'report.json'), 'utf8'));
  const router = report.probes.find(p => p.endpointId === 'text:openrouter');
  assert.equal(router.pagination.completeness, 'partial');
  assert.equal(router.pagination.pagesFetched, 1);
  assert.equal(router.inventory.excluded.length, 3);
  assert.equal(report.probes.find(p => p.endpointId === 'text:siliconflow').inventory.newIds.includes('fixture/new-chat'), true);
  assert.equal(report.probes.find(p => p.endpointId === 'image:openrouter-images').reason, 'invalid_catalog_payload');
  assert.equal(report.coverage.completeCatalogEndpoints.length, 0);
  assert.equal(report.completedProbeCount, 2);
  assert.equal(JSON.parse(stdout).hasInventoryCandidates, true);
  assert.equal((await readFile(join(out, 'report.md'), 'utf8')).includes('partial'), true);
} finally {
  await rm(testDir, { recursive: true, force: true });
}
process.stdout.write('inventory, pagination and CLI coverage regressions passed\n');

assert.equal(summarizeDiscovery([{endpointId:'offline',status:'error'}]).inventoryStatus, 'unavailable');
assert.equal(summarizeDiscovery([{endpointId:'offline',status:'error'}]).requiresInventoryReview, true);
assert.equal(summarizeDiscovery([]).inventoryStatus, 'not_requested');
