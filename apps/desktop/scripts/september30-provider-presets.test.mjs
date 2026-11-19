import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

const { build } = createRequire(import.meta.resolve('vite'))('esbuild');
async function loadPresets(filename) {
  const compiled = await build({ entryPoints: [fileURLToPath(new URL(filename, import.meta.url))], bundle: true, platform: 'node', format: 'esm', write: false });
  return import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);
}
const { findProviderModelPreset: find, findProviderPreset, isRemovedProviderModel, retiredProviderTypes } = await loadPresets('../src/lib/providerPresets.ts');

test('native models project current limits and preserve gated preview identity in the picker', () => {
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
  const stable = find({ provider: 'open_ai', baseUrl: 'https://api.minimax.io/v1', model: 'MiniMax-M3.1-Flash' });
  assert.equal(stable.descriptor.access, 'public');
  assert.equal(stable.capabilities.reasoning.mode, 'always');
  assert.deepEqual(stable.capabilities.reasoning.effortLevels, ['low', 'medium', 'high', 'xhigh', 'max']);
});

test('gateway reasoning controls and regional hosted models do not leak into direct or subscription routes', () => {
  const router = { provider: 'openrouter', baseUrl: 'https://openrouter.ai/api/v1' };
  const glm = find({ ...router, model: 'z-ai/glm-5.3-prime' });
  assert.deepEqual(glm.capabilities.reasoning.effortLevels, ['low', 'high', 'max']);
  const qwen = find({ ...router, model: 'qwen/qwen3.8-max-prime' });
  assert.equal(qwen.capabilities.reasoning.mode, 'always');
  assert.equal(qwen.capabilities.reasoning.defaultEffort, 'xhigh');
  assert.equal(find({ ...router, model: 'upstage/solar-mini4' }).capabilities.reasoning.defaultEnabled, false);
  assert.equal(find({ ...router, model: 'upstage/solar-mini4' }).capabilities.reasoning.defaultEffort, 'medium');
  const cn = { provider: 'alibaba_model_studio', baseUrl: 'https://work.cn-beijing.maas.aliyuncs.com/compatible-mode/v1' };
  const sg = { ...cn, baseUrl: 'https://work.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1' };
  assert.equal(find({ ...cn, model: 'ZHIPU/GLM-5.3-FlashX' }).descriptor.limits.maxOutputTokens, 131_072);
  assert.equal(find({ ...sg, model: 'ZHIPU/GLM-5.3-FlashX' }), null);
  assert.equal(find({ ...sg, model: 'deepseek-v4.1-flash' }).descriptor.limits.maxOutputTokens, 393_216);
  const hostedDeepSeek = find({ ...cn, model: 'vanchin/deepseek-v4.1-flash' });
  assert.equal(hostedDeepSeek.descriptor.access, 'account_enablement');
  assert.deepEqual(hostedDeepSeek.capabilities.reasoning.effortLevels, ['low', 'high', 'max']);
  assert.equal(find({ ...sg, model: hostedDeepSeek.id }), null);
  const hostedMimo = find({ ...cn, model: 'xiaomi/mimo-v2.6-pro-ultraspeed' });
  assert.equal(hostedMimo.descriptor.limits.contextTokens, 1_048_576);
  assert.equal(hostedMimo.capabilities.reasoning, null);
  const plan = find({ provider: 'qwen', baseUrl: 'https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1', model: 'deepseek-v4.1-flash' });
  assert.ok(plan, 'Token Plan added this exact model in the October catalog');
  assert.equal(plan.descriptor.limits.maxOutputTokens, null, 'do not copy a PAYG output limit into Token Plan');
  const snapshot = find({ provider: 'qwen', baseUrl: 'https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1', model: 'deepseek-v4.1-flash-1017' });
  assert.ok(snapshot);
  assert.equal(snapshot.descriptor.limits.maxOutputTokens, null);
});

test('confirmed retirements disappear while still-supported legacy and future shutdowns remain', () => {
  const yi = findProviderPreset({ provider: 'yi', baseUrl: 'https://api.lingyiwanwu.com/v1' });
  assert.equal(yi.models.length, 0);
  assert.equal(retiredProviderTypes.has('yi'), true);
  assert.equal(retiredProviderTypes.has('open_ai'), false);
  assert.equal(retiredProviderTypes.has('custom'), false);
  for (const id of ['yi-large', 'yi-medium', 'yi-spark', 'yi-large-turbo']) assert.equal(isRemovedProviderModel('yi', id), true);
  for (const id of ['doubao-seed-code-preview-251028', 'doubao-seed-1-6-251015', 'doubao-seed-1-6-flash-250828']) assert.equal(isRemovedProviderModel('doubao', id), true);
  assert.equal(isRemovedProviderModel('doubao', 'doubao-seed-2-0-pro-260215'), false);
  assert.ok(find({ provider: 'google', model: 'gemini-2.5-pro' }));
});

test('Ling 3.1 Flash keeps its Beijing activation and toggle-only reasoning contract', () => {
  const model = 'antdigital/Ling-3.1-flash';
  const ling = find({ provider: 'alibaba_model_studio', baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1', model });
  assert.equal(ling.descriptor.limits.contextTokens, 1_048_576);
  assert.equal(ling.descriptor.limits.maxOutputTokens, 262_144);
  assert.equal(ling.descriptor.access, 'account_enablement');
  assert.equal(ling.capabilities.vision, false);
  assert.equal(ling.capabilities.reasoning.defaultEnabled, true);
  assert.equal(ling.capabilities.reasoning.thinkingBudget, undefined);
  assert.equal(ling.capabilities.reasoning.effortLevels, undefined);
  for (const baseUrl of ['https://dashscope-intl.aliyuncs.com/compatible-mode/v1', 'https://private.example/v1']) {
    assert.equal(find({ provider: 'alibaba_model_studio', baseUrl, model }), null);
  }
});

test('embedding models keep regional vector dimensions and batch limits', async () => {
  const { EMBEDDING_PROVIDER_PRESETS: presets, findEmbeddingProviderPreset } = await loadPresets('../src/lib/embeddingProviderPresets.ts');
  const qwen = presets.find(preset => preset.id === 'alibaba-model-studio-cn').models;
  assert.deepEqual(qwen.find(model => model.id === 'qwen3.7-text-embedding').allowedDimensions, [256, 512, 768, 1024, 1536, 2048, 2560]);
  assert.equal(qwen.find(model => model.id === 'qwen3.7-text-embedding-flash').maxBatchSize, 20);
  const workspace = host => `https://${host}.maas.aliyuncs.com/compatible-mode/v1`;
  const singapore = findEmbeddingProviderPreset(workspace('example.ap-southeast-1'));
  assert.equal(singapore.id, 'alibaba-model-studio-sg');
  assert.deepEqual(singapore.models.find(model => model.id === 'text-embedding-v3').allowedDimensions, [512, 768, 1024]);
  assert.deepEqual(findEmbeddingProviderPreset(workspace('example.cn-hongkong')).models.map(model => model.id), ['text-embedding-v4']);
  assert.equal(findEmbeddingProviderPreset(workspace('token-plan.cn-beijing')), null);
  for (const id of ['embed-v5.0-pro', 'embed-v5.0-fast']) {
    const model = presets.find(preset => preset.id === 'cohere').models.find(model => model.id === id);
    assert.equal(model.dimensions, 2048);
    assert.ok(model.allowedDimensions.includes(1536));
  }
});
