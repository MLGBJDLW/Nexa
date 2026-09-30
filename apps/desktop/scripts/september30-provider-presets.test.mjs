import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';

const { build } = createRequire(import.meta.resolve('vite'))('esbuild');
const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/lib/providerPresets.ts', import.meta.url))], bundle: true, platform: 'node', format: 'esm', write: false });
const { findProviderModelPreset: find, findProviderPreset, isRemovedProviderModel } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);

test('September additions project actual model limits and gated access into the picker', () => {
  const sol = find({ provider: 'open_ai', baseUrl: 'https://api.openai.com/v1', model: 'gpt-6.1-sol' });
  assert.equal(sol.descriptor.limits.contextTokens, 1_050_000);
  assert.equal(sol.descriptor.limits.maxOutputTokens, 128_000);
  assert.equal(sol.capabilities.reasoning.mode, 'always');
  assert.deepEqual(sol.capabilities.reasoning.effortLevels, ['low', 'medium', 'high', 'xhigh', 'max']);
  assert.equal(find({ provider: 'open_ai', baseUrl: 'https://private.example/v1', model: sol.id }), null);
  const minimax = find({ provider: 'open_ai', baseUrl: 'https://api.minimax.io/v1', model: 'MiniMax-M3.1-Flash-Preview' });
  assert.equal(minimax.descriptor.access, 'account_enablement');
  assert.equal(minimax.descriptor.limits.maxOutputTokens, 524_288);
  assert.deepEqual(minimax.descriptor.inputModalities, ['text', 'image']);
  assert.equal(find({ provider: 'open_ai', baseUrl: 'https://api.openai.com/v1', model: minimax.id }), null);
});

test('gateway reasoning controls and regional hosted models do not leak into direct or subscription routes', () => {
  const router = { provider: 'openrouter', baseUrl: 'https://openrouter.ai/api/v1' };
  const glm = find({ ...router, model: 'z-ai/glm-5.3-prime' });
  assert.deepEqual(glm.capabilities.reasoning.effortLevels, ['low', 'high', 'max']);
  const qwen = find({ ...router, model: 'qwen/qwen3.8-max-prime' });
  assert.equal(qwen.capabilities.reasoning.mode, 'always');
  assert.equal(qwen.capabilities.reasoning.defaultEffort, 'xhigh');
  assert.equal(find({ ...router, model: 'upstage/solar-mini4' }).capabilities.reasoning.defaultEffort, 'none');
  const cn = { provider: 'alibaba_model_studio', baseUrl: 'https://work.cn-beijing.maas.aliyuncs.com/compatible-mode/v1' };
  const sg = { ...cn, baseUrl: 'https://work.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1' };
  assert.equal(find({ ...cn, model: 'ZHIPU/GLM-5.3-FlashX' }).descriptor.limits.maxOutputTokens, 131_072);
  assert.equal(find({ ...sg, model: 'ZHIPU/GLM-5.3-FlashX' }), null);
  assert.equal(find({ ...sg, model: 'deepseek-v4.1-flash' }).descriptor.limits.maxOutputTokens, 393_216);
  assert.equal(find({ provider: 'qwen', baseUrl: 'https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1', model: 'deepseek-v4.1-flash' }), null);
});

test('confirmed retirements disappear while still-supported legacy and future shutdowns remain', () => {
  const yi = findProviderPreset({ provider: 'yi', baseUrl: 'https://api.lingyiwanwu.com/v1' });
  assert.equal(yi.models.length, 0);
  for (const id of ['yi-large', 'yi-medium', 'yi-spark', 'yi-large-turbo']) assert.equal(isRemovedProviderModel('yi', id), true);
  for (const id of ['doubao-seed-code-preview-251028', 'doubao-seed-1-6-251015', 'doubao-seed-1-6-flash-250828']) assert.equal(isRemovedProviderModel('doubao', id), true);
  assert.equal(isRemovedProviderModel('doubao', 'doubao-seed-2-0-pro-260215'), false);
  assert.ok(find({ provider: 'google', model: 'gemini-2.5-pro' }));
});

test('new embedding models keep model-specific vector dimensions and batch limits', () => {
  const presets = JSON.parse(readFileSync(new URL('../../../shared/embedding-provider-presets.json', import.meta.url), 'utf8'));
  const qwen = presets.find(preset => preset.id === 'alibaba-model-studio-cn').models;
  assert.deepEqual(qwen.find(model => model.id === 'qwen3.7-text-embedding').allowedDimensions, [256, 512, 768, 1024, 1536, 2048, 2560]);
  assert.equal(qwen.find(model => model.id === 'qwen3.7-text-embedding-flash').maxBatchSize, 20);
  for (const id of ['embed-v5.0-pro', 'embed-v5.0-fast']) {
    const model = presets.find(preset => preset.id === 'cohere').models.find(model => model.id === id);
    assert.equal(model.dimensions, 2048);
    assert.ok(model.allowedDimensions.includes(1536));
  }
});
