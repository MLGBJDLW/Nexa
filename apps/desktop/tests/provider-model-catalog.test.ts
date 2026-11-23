import { catalogModelsForSnapshot, PROVIDER_PRESETS } from '../src/lib/providerPresets';
import { selectImplicitDefault } from '../src/lib/modelCatalog';
import { savedConnectionModelFeatures } from '../src/lib/connectionModelFeatures';
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
  const sol = nativeModels.find(model => model.id === 'gpt-6.2-sol')!.descriptor;
  previous.descriptors = [{ ...sol, lifecycle: 'deprecated', recommended: false, deprecationDate: '2999-01-01' }];
  previous.tombstones = [{ ...sol, id: 'gpt-5.2-pro-2025-12-11', lifecycle: 'removed', aliases: [] }];
  const announced = catalogModelsForSnapshot(previous);
  assertEqual(announced.find(model => model.id === sol.id)?.recommended, false, 'an announced shutdown disables recommendation');
  assert(announced.some(model => model.id === 'gpt-5.2-pro'), 'a retired snapshot leaves the rolling model selectable');
  previous.descriptors[0].deprecationDate = '2000-01-01';
  assert(!catalogModelsForSnapshot(previous).some(model => model.id === sol.id), 'a stored shutdown survives offline picker refresh');
}

testPreviousReleaseCatalog();

{
  const route = { provider: 'open_ai', baseUrl: 'https://api.openai.com/v1', model: 'gpt-6.2-sol' };
  const descriptor = PROVIDER_PRESETS.find(preset => preset.id === 'openai')!.models.find(item => item.id === route.model)!.descriptor;
  const ids = (connection: typeof route, platform: 'phone' | 'desktop', selected?: typeof descriptor) =>
    savedConnectionModelFeatures(connection, platform, selected).map(feature => feature.id).sort();
  assertDeepEqual(ids(route, 'phone'), ['chat', 'imageInput', 'nativeWebSearch'], 'standalone phone exposes its own model-backed operations');
  assertDeepEqual(ids(route, 'desktop'), ['chat', 'conversationSummary', 'imageInput', 'incrementalLive', 'nativeLive', 'nativeWebSearch'],
    'desktop exposes both Live paths and configurable summarization');
  assertDeepEqual(ids(route, 'desktop', { ...descriptor, availableToCredential: false }), ['nativeLive'],
    'a separate native Live model can be selected when the chat model is unavailable');
  for (const baseUrl of ['https://relay.example/v1', 'https://user@api.openai.com/v1', 'https://api.openai.com/v1?']) {
    assertDeepEqual(ids({ ...route, baseUrl }, 'phone', descriptor), ['chat'], 'an edited endpoint cannot inherit native search or vision');
  }
  assertDeepEqual(ids({ ...route, model: 'custom-chat' }, 'phone', descriptor), ['chat'], 'a different model cannot supply capabilities');
  assert(ids({ ...route, baseUrl: 'https://openrouter.ai/api/v1', provider: 'openrouter', model: 'custom-chat' }, 'phone').includes('nativeWebSearch'),
    'OpenRouter provides its server-side tool on the route');
  assert(ids({ provider: 'google', baseUrl: 'https://generativelanguage.googleapis.com/v1beta', model: 'models/gemini-4-hydrogen' }, 'phone').includes('nativeWebSearch'),
    'the Google model resource prefix is supported by the adapter');
  const workspace = { provider: 'qwen', baseUrl: 'https://ws-a.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1', model: 'qwen4-max' };
  assert(ids(workspace, 'desktop').includes('nativeLive'), 'Singapore workspace PAYG connects to native Live');
  assert(!ids({ ...workspace, baseUrl: workspace.baseUrl.replace('ap-southeast-1', 'cn-hongkong') }, 'desktop').includes('nativeLive'), 'Hong Kong has no native Live route');
  assertDeepEqual(ids({ ...route, provider: 'openai_codex' }, 'desktop'), [], 'a subscription session cannot supply API-key features');
}

for (const preset of PROVIDER_PRESETS.filter(preset => !preset.runtime)) {
  const first = preset.models[0];
  const selected = selectImplicitDefault(preset.models);
  if (!first || !preset.requiresApiKey || first.descriptor.access !== 'public') {
    assertEqual(selected, null, `${preset.id} requires explicit installation or account access`);
    continue;
  }
  assert(first.recommended, `${preset.id} should show its recommendation first`);
  assertEqual(selected?.id, first.id, `${preset.id} should preselect its public recommendation`);
  const denied = { ...first, descriptor: { ...first.descriptor, availableToCredential: false } };
  assert(selectImplicitDefault([denied, ...preset.models.slice(1)])?.id !== first.id,
    `${preset.id} must honor a negative account discovery`);
}
