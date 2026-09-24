import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

// Resolve through Vite, which owns the esbuild dependency under strict pnpm.
const { build } = createRequire(import.meta.resolve('vite'))('esbuild');

// Bundle the actual UI catalog (including its shared JSON) as Vite does.
const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/lib/providerPresets.ts', import.meta.url))],
  bundle: true,
  platform: 'node',
  format: 'esm',
  write: false,
});
const { findProviderPreset, findProviderModelPreset } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`
);

test('Qwen September models retain region, alias, vision and effort controls in the UI catalog', () => {
  for (const [baseUrl, presetId, region] of [
    ['https://dashscope.aliyuncs.com/compatible-mode/v1', 'alibaba-model-studio', 'cn-beijing'],
    ['https://workspace123.cn-beijing.maas.aliyuncs.com/compatible-mode/v1', 'alibaba-model-studio', 'cn-beijing'],
    ['https://dashscope-intl.aliyuncs.com/compatible-mode/v1', 'qwen-cloud-intl', 'ap-southeast-1'],
    ['https://workspace123.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1', 'qwen-cloud-intl', 'ap-southeast-1'],
  ]) {
    const route = { provider: 'alibaba_model_studio', baseUrl };
    assert.equal(findProviderPreset(route)?.id, presetId);
    const max = findProviderModelPreset({ ...route, model: 'qwen3.8-max-2026-09-02' });
    assert.equal(max?.id, 'qwen3.8-max-0902');
    assert.equal(max?.descriptor.limits.contextTokens, 1_000_000);
    assert.equal(max?.descriptor.limits.maxOutputTokens, 131_072);
    assert.deepEqual(max?.descriptor.regions, [region]);
    const omni = findProviderModelPreset({ ...route, model: 'qwen3.8-omni-flash' });
    assert.deepEqual(omni?.descriptor.inputModalities, ['text', 'image', 'audio', 'video']);
    assert.deepEqual(omni?.descriptor.outputModalities, ['text']);
    assert.equal(omni?.descriptor.limits.contextTokens, 1_000_000);
    assert.equal(omni?.capabilities.vision, true);
    assert.equal(omni?.capabilities.reasoning.mode, 'optional');
    assert.equal(omni?.capabilities.reasoning.defaultEffort, 'low');
    assert.equal(omni?.capabilities.reasoning.thinkingBudget.enabled, false);
  }
});

test('Qwen PAYG model metadata cannot leak into subscription, trial, or private routes', () => {
  for (const baseUrl of [
    'https://trial.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1',
    'https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1',
    'https://workspace123.ap-southeast-1.maas.aliyuncs.com.evil.example/compatible-mode/v1',
    'https://workspace123.ap-southeast-1.maas.aliyuncs.com:8443/compatible-mode/v1',
    'https://workspace123.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1?other=1',
  ]) {
    assert.equal(findProviderPreset({ provider: 'alibaba_model_studio', baseUrl }), null);
  }
  assert.equal(findProviderModelPreset({
    provider: 'qwen',
    baseUrl: 'https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1',
    model: 'qwen3.8-omni-flash',
  }), null);
});
