import assert from 'node:assert/strict';
import {
  compareEndpointModels,
  compareCatalogInventory,
  fetchCatalogPages,
  googleCatalogMethod,
  supportsCatalogOperation,
  driftDetected,
  normalizeCatalogModel,
  normalizeDiscoveredModel,
  normalizeGoogleCatalog,
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

const resolvedAliases = compareEndpointModels({ models: [
  { id:'jev-latest', discoveryId:'~typesafe/jev-latest', capabilities:{} },
  { id:'~typesafe/jev-latest', capabilities:{} },
] }, [{ id:'~typesafe/jev-latest' }]);
assert.deepEqual(resolvedAliases.missingIds, []);
assert.deepEqual(resolvedAliases.newIds, []);
const router = { endpointId: 'text:openrouter', models: [{
  id: 'google/gemini-4-carbon', lifecycle: 'active',
  capabilities: { toolCalling: true, reasoning: { mode: 'optional', defaultEnabled: false } },
  limits: { contextTokens: 100 },
}] };
const liveRouter = [
  { id: 'google/gemini-4-carbon', context_length: 200,
    supported_parameters: ['tools', 'reasoning'], reasoning: { mandatory: false, default_enabled: true },
    architecture: { output_modalities: ['text'] }, endpoints: ['/api/v1/chat/completions'] },
  { id: 'google/a-future-gemini', architecture: { output_modalities: ['text'] }, endpoints: ['/api/v1/chat/completions'] },
  { id: 'google/batch-only:batch', architecture: { output_modalities: ['text'] }, endpoints: ['/api/v1/batches'] },
  { id: 'google/image-only', architecture: { output_modalities: ['image'] } },
];
const newRouter = compareCatalogInventory(router, liveRouter);
assert.deepEqual(newRouter.newIds, ['google/a-future-gemini']);
assert.deepEqual(newRouter.missingIds, []);
assert.deepEqual(newRouter.capabilityChanged, [{ id: 'google/gemini-4-carbon', fields: ['reasoning.defaultEnabled'] }]);
assert.deepEqual(newRouter.limitChanged, [{ id: 'google/gemini-4-carbon', fields: ['contextTokens'] }]);
assert.equal(driftDetected(newRouter), true);
const newlyDeclared = compareCatalogInventory({ endpointId: 'text:openrouter', models: [{ id: 'existing-id', capabilities: {}, limits: {} }] }, [{
  id: 'existing-id', supported_parameters: ['tools', 'reasoning', 'reasoning_effort'], context_length: 65536,
  reasoning: { mandatory: true, supported_efforts: ['low', 'high'], default_effort: 'high', supports_max_tokens: false },
}]);
assert.ok(newlyDeclared.capabilityChanged[0].fields.includes('reasoning'));
assert.ok(newlyDeclared.capabilityChanged[0].fields.includes('toolCalling'));
assert.deepEqual(newlyDeclared.limitChanged, [{ id: 'existing-id', fields: ['contextTokens'] }]);
assert.equal(newlyDeclared.parameterChanged.length, 1);
const reasoningChanged = compareCatalogInventory({ endpointId: 'text:openrouter', models: [{
  id: 'existing-id', capabilities: { reasoning: { effortLevels: ['high', 'low'], defaultEffort: 'low', thinkingBudget: { enabled: true } } },
}] }, [{ id: 'existing-id', reasoning: { supported_efforts: ['low', 'high', 'xhigh'], default_effort: 'high', supports_max_tokens: false } }]);
assert.deepEqual(reasoningChanged.capabilityChanged[0].fields, ['reasoning.defaultEffort', 'reasoning.effortLevels', 'reasoning.thinkingBudget.enabled']);
const reasoningWithdrawn = compareCatalogInventory(router, [{
  id: router.models[0].id, supported_parameters: ['tools'], reasoning: null,
}]);
assert.ok(reasoningWithdrawn.capabilityChanged[0].fields.includes('reasoning'), 'a complete parameter list can withdraw stale reasoning controls');
assert.equal(normalizeDiscoveredModel('text:openrouter', { id: 'plain', supported_parameters: [] }).capabilities.reasoning, null);
assert.equal(normalizeDiscoveredModel('text:openrouter', {
  id: 'mandatory', supported_parameters: [], reasoning: { mandatory: true },
}).capabilities.reasoning.mode, 'always', 'explicit intrinsic reasoning survives a lack of configurable parameters');
for (const invalid of [undefined, null, 'reasoning', [null], ['tools', {}]]) {
  const normalized = normalizeDiscoveredModel('text:openrouter', {
    id: 'unknown', supported_parameters: invalid,
    architecture: { input_modalities: 4, output_modalities: 'text' },
  });
  assert.equal(Object.hasOwn(normalized.capabilities, 'reasoning'), false, 'unknown parameters do not establish a capability withdrawal');
  assert.equal(Object.hasOwn(normalized.capabilities, 'vision'), false);
}
const googleRows = [
  { name: 'models/gemini-chat', supportedGenerationMethods: ['generateContent'], inputTokenLimit: 1048576, outputTokenLimit: 65536 },
  { name: 'models/gemini-embedding', supportedGenerationMethods: ['embedContent'], inputTokenLimit: 8192 },
  { name: 'models/unknown', supportedGenerationMethods: 'embedContent' },
];
const googleEmbedding = normalizeGoogleCatalog({ endpointId: 'embedding:google-gemini' }, googleRows);
assert.deepEqual(googleEmbedding.map(model => model.id), ['gemini-embedding']);
assert.equal(googleEmbedding[0].context_length, 8192);
assert.deepEqual(googleEmbedding[0].output_modalities, ['embeddings']);
assert.deepEqual(compareCatalogInventory({ endpointId: 'embedding:google-gemini', models: [{ id: 'gemini-embedding' }] }, googleEmbedding).missingIds, []);
const googleChat = normalizeGoogleCatalog({ endpointId: 'text:google' }, googleRows);
assert.deepEqual(googleChat.map(model => model.id), ['gemini-chat']);
assert.equal(googleChat[0].top_provider.max_completion_tokens, 65536);
for (const surface of ['image', 'speech_to_text', 'text_to_speech']) {
  const endpoint = { endpointId: `${surface}:google-gemini` };
  assert.equal(googleCatalogMethod(endpoint), null);
  assert.equal(normalizeGoogleCatalog(endpoint, googleRows), null,
    'text model listings do not assert image or speech model support');
}
for (const provider of ['open_ai', 'siliconflow']) {
  for (const surface of ['image', 'embedding', 'speech_to_text', 'text_to_speech', 'decision', 'video']) {
    const endpoint = { endpointId: `${surface}:${provider}`, provider, baseUrl: 'https://catalog.example/v1', models: [] };
    assert.equal(supportsCatalogOperation(endpoint), false,
      'generic authenticated inventories cannot establish an operation contract');
    assert.deepEqual(compareCatalogInventory(endpoint, [{ id: 'gpt-6-sol' }]).newIds, []);
  }
}
assert.equal(supportsCatalogOperation({ endpointId: 'embedding:openrouter' }, '/embeddings/models'), true);
assert.equal(supportsCatalogOperation({ endpointId: 'embedding:google-gemini', provider: 'google', baseUrl: 'https://generativelanguage.googleapis.com/v1beta' }), true);
assert.deepEqual(compareCatalogInventory({ endpointId: 'embedding:jina', models: [] }, [{ id: 'jina-ai/untyped-unknown-model' }]).newIds, []);
assert.deepEqual(compareCatalogInventory({ endpointId: 'video:openrouter-video', operationSpecific: true, models: [] }, [{ id: 'new-video-model' }]).newIds, ['new-video-model']);
const retiredOnly = compareCatalogInventory({ ...router, models: [{ ...router.models[0], lifecycle: 'removed' }] }, []);
assert.equal(driftDetected(retiredOnly), false, 'existing tombstones are not repeated missing-model alerts');
const denseEmbeddings = compareCatalogInventory({ endpointId: 'embedding:jina', models: [{
  id: 'jina-embeddings-v4', inputModalities: ['text'], outputModalities: ['embedding'], capabilities: { vision: false },
}] }, [
  { id: 'jina-ai/jina-embeddings-v4', input_modalities: ['text', 'image'], output_modalities: ['embeddings'] },
  { id: 'jina-ai/jina-colbert-v2', input_modalities: ['text'], output_modalities: ['embeddings'] },
]);
assert.equal(driftDetected(denseEmbeddings), false, 'unsupported vector shape and unimplemented modality expansion do not create permanent drift');
const requests = [];
const pagedRows = await fetchCatalogPages('https://catalog.example/models?output_modalities=text', {
  fetchImpl: async url => {
    requests.push(url);
    return { ok: true, json: async () => requests.length === 1
      ? { data: [{ id: 'page-one' }], total_count: 2, links: { next: '/models?offset=1&output_modalities=text' } }
      : { data: [{ id: 'new-on-page-two' }], total_count: 2 } };
  },
});
assert.deepEqual(pagedRows.map(model => model.id), ['page-one', 'new-on-page-two']);
assert.equal(requests.length, 2);
await assert.rejects(fetchCatalogPages('https://catalog.example/models', {
  fetchImpl: async () => ({ ok: true, json: async () => ({ data: [], links: { next: 'https://foreign.example/models' } }) }),
}), /invalid_pagination/);
await assert.rejects(fetchCatalogPages('https://catalog.example/models', {
  fetchImpl: async () => ({ ok: true, json: async () => ({ error: 'not a catalog' }) }),
}), /invalid_catalog/);
await assert.rejects(fetchCatalogPages('https://catalog.example/models', {
  fetchImpl: async () => ({ ok: true, json: async () => ({ data: [{ id: 'incomplete' }], total_count: 2 }) }),
}), /incomplete_catalog/);
process.stdout.write('model catalog audit fixtures passed\n');

assert.deepEqual(normalizeCatalogModel('video', { modelId: 'MiniMax-H3', displayName: 'MiniMax H3', releaseStatus: 'ga', selectable: true }), { modelId: 'MiniMax-H3', displayName: 'MiniMax H3', releaseStatus: 'ga', selectable: true, id: 'MiniMax-H3', name: 'MiniMax H3', status: 'active' });
assert.equal(normalizeCatalogModel('video', { modelId: 'gated', displayName: 'Gated', releaseStatus: 'unverified' }).status, 'gated');
