import { catalogModelsForSnapshot } from '../src/lib/providerPresets';
import {
  bindProviderModelCatalogCredential,
  catalogMatchesProvider,
  isProviderModelCatalogStale,
  loadProviderModelCatalog,
  providerModelCatalogCacheKey,
  saveProviderModelCatalog,
  type ProviderModelCatalogSnapshot,
} from '../src/lib/providerModelCatalog';

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertEqual<T>(actual: T, expected: T, message: string): void {
  assert(Object.is(actual, expected), `${message}: expected ${String(expected)}, got ${String(actual)}`);
}

function assertDeepEqual(actual: unknown, expected: unknown, message: string): void {
  assert(JSON.stringify(actual) === JSON.stringify(expected), message);
}

function snapshot(): ProviderModelCatalogSnapshot {
  return {
    provider: 'alibaba_model_studio',
    baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1/',
    refreshedAt: '2026-07-31T08:00:00Z',
    liveDiscoverySucceeded: true,
    models: [
      {
        id: 'account-model',
        name: 'account-model',
        recommended: false,
        source: 'discovered',
        status: 'active',
        regions: ['cn-beijing'],
        lastVerifiedAt: '2026-07-31T08:00:00Z',
        modalities: ['text'],
        supportsTools: false,
        supportsStructuredOutput: false,
        reasoningEfforts: [],
      },
    ],
  };
}

function testCatalogCache(): void {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); },
  };
  const value = snapshot();
  const apiKey = 'account-one-secret';
  const boundValue = bindProviderModelCatalogCredential(value, apiKey);

  saveProviderModelCatalog(boundValue, apiKey, storage);

  assertEqual(
    providerModelCatalogCacheKey(value.provider, value.baseUrl, apiKey),
    providerModelCatalogCacheKey(value.provider, value.baseUrl?.replace(/\/$/, ''), apiKey),
    'cache key should normalize a trailing slash',
  );
  assertDeepEqual(
    loadProviderModelCatalog(value.provider, value.baseUrl?.replace(/\/$/, ''), apiKey, storage),
    boundValue,
    'catalog should restore the cached live snapshot',
  );
  assertEqual(
    loadProviderModelCatalog('open_ai', value.baseUrl, apiKey, storage),
    null,
    'catalog cache should be isolated by provider',
  );
  assertEqual(
    loadProviderModelCatalog(value.provider, value.baseUrl, 'account-two-secret', storage),
    null,
    'catalog cache should be isolated by credential',
  );
}

function testCatalogFreshness(): void {
  const value = snapshot();
  const boundValue = bindProviderModelCatalogCredential(value, 'account-secret');
  assertEqual(
    catalogMatchesProvider(boundValue, 'alibaba_model_studio', value.baseUrl, 'account-secret'),
    true,
    'matching provider and endpoint should be accepted',
  );
  assertEqual(
    catalogMatchesProvider(boundValue, 'open_ai', value.baseUrl, 'account-secret'),
    false,
    'different providers should not share snapshots',
  );
  assertEqual(
    isProviderModelCatalogStale(value, Date.parse('2026-08-01T07:59:59Z')),
    false,
    'snapshot should remain fresh through the TTL',
  );
  assertEqual(
    isProviderModelCatalogStale(value, Date.parse('2026-08-01T08:00:01Z')),
    true,
    'snapshot should become stale after the TTL',
  );
}

testCatalogCache();
testCatalogFreshness();

function testPreviousReleaseCatalog(): void {
  const previous = snapshot();
  previous.provider = 'openrouter';
  previous.baseUrl = 'https://openrouter.ai/api/v1';
  const models = catalogModelsForSnapshot(previous);
  for (const id of ['google/gemini-4-carbon', 'google/gemini-4-argon', 'google/gemini-4-hydrogen', 'anthropic/claude-fable-5.2']) {
    const model = models.find(model => model.id === id);
    assert(model, `an old account snapshot must not hide bundled ${id}`);
    assertEqual(model.descriptor.availableToCredential, null, 'new bundled IDs have unknown account access');
  }
  assert(models.some(model => model.id === 'account-model'), 'retain account-discovered IDs through upgrades');
  const carbon = models.find(model => model.id === 'google/gemini-4-carbon')!;
  const oldDescriptor = { ...carbon.descriptor, availableToCredential: false, lastVerifiedAt: '2026-07-30',
    capabilities: {}, aliases: undefined } as unknown as typeof carbon.descriptor;
  previous.descriptors = [oldDescriptor];
  const upgraded = catalogModelsForSnapshot(previous).find(model => model.id === carbon.id)!;
  assert(upgraded.capabilities?.reasoning, 'a newer bundled contract supersedes stale reasoning metadata');
  assertEqual(upgraded.descriptor.availableToCredential, false, 'retain account-specific availability');
  previous.descriptors = [{ ...oldDescriptor, lifecycle: 'removed' }];
  assert(!catalogModelsForSnapshot(previous).some(model => model.id === carbon.id), 'legacy descriptors can contain retirement evidence');
  previous.descriptors = undefined;
  previous.baseUrl = 'https://tenant.example/v1';
  assertEqual(catalogModelsForSnapshot(previous).length, 1, 'never borrow official models for a custom endpoint');
  previous.models.push({ ...previous.models[0], id: 'retired', aliases: ['account-model'], status: 'removed' });
  assertEqual(catalogModelsForSnapshot(previous).length, 0, 'legacy custom-endpoint tombstones exclude retired aliases');

  previous.provider = 'open_ai';
  previous.baseUrl = 'https://api.openai.com/v1';
  previous.models = [{ ...snapshot().models[0], id: 'gpt-5.6', recommended: true }];
  const nativeModels = catalogModelsForSnapshot(previous);
  assertDeepEqual(nativeModels.slice(0, 2).map(model => model.id), ['gpt-6.2-sol', 'gpt-6.2-astra'],
    'an existing account receives the current release ordering');
  assertEqual(nativeModels.find(model => model.id === 'gpt-5.6')?.recommended, false,
    'a previous release recommendation must not shadow the bundled recommendation');
}

testPreviousReleaseCatalog();
