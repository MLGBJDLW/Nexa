import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

const { build } = createRequire(import.meta.resolve('vite'))('esbuild');
const compiled = await build({
  stdin: {
    contents: `export * from './providerPresets.ts';
      export * from './reasoningControls.ts';
      export * from './providerModelCatalog.ts';`,
    resolveDir: fileURLToPath(new URL('../src/lib', import.meta.url)),
  },
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { PROVIDER_PRESETS, getReasoningCapability, reasoningOffLabelKey,
  defaultReasoningEffort, catalogModelsForSnapshot } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`
);

for (const [provider, id, offMode] of [
  ['anthropic', 'claude-sonnet-5-5', 'between_tools'],
  ['openrouter', 'anthropic/claude-sonnet-5.5', undefined],
  ['openrouter', '~anthropic/claude-sonnet-latest', undefined],
]) {
  test(`Sonnet 5.5 UI catalog and hydration respect ${provider}/${id}`, () => {
    const preset = PROVIDER_PRESETS.find(preset => preset.id === provider);
    const model = preset.models.find(model => model.id === id);
    assert.ok(model, 'model must be selectable');
    assert.equal(model.descriptor.limits.contextTokens, 1_000_000);
    assert.equal(model.descriptor.limits.maxOutputTokens, 128_000);
    assert.ok(model.descriptor.availableToCredential == null, 'documentation does not prove account access');
    const reasoning = getReasoningCapability({ provider, baseUrl: preset.baseUrl, model: id });
    assert.equal(reasoning?.disabledMode, offMode);
    assert.equal(defaultReasoningEffort(reasoning), 'high');
    assert.deepEqual(reasoning?.effortLevels, ['low', 'medium', 'high', 'xhigh', 'max']);
    assert.equal(reasoning?.thinkingBudget?.enabled, false);
    assert.equal(reasoning?.mode, provider === 'anthropic' ? 'optional' : 'always');
    assert.equal(reasoningOffLabelKey(reasoning), offMode ? 'settings.reasoningBetweenTools' : 'settings.reasoningNone');
    const hydrated = catalogModelsForSnapshot({ provider, baseUrl: preset.baseUrl,
      refreshedAt: '2026-09-29', liveDiscoverySucceeded: false, models: [], descriptors: [model.descriptor] });
    assert.equal(hydrated[0].descriptor.capabilities.reasoning?.disabledMode, offMode);
    assert.equal(getReasoningCapability({ provider, baseUrl: 'https://private.example/v1', model: id }), null);
  });
}
