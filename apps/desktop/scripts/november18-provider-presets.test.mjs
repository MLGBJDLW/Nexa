import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
const read = name => JSON.parse(readFileSync(new URL(`../../../shared/${name}.json`, import.meta.url), 'utf8'));
const text = read('provider-presets');
const provider = id => text.find(p => p.id === id);
test('GPT6.2 native exact IDs stay always-reasoning Responses routes, not router Pro aliases', () => {
  for (const id of ['gpt-6.2-astra', 'gpt-6.2-sol']) {
    const m = provider('openai').models.find(m => m.id === id);
    assert.equal(m.recommended, id === 'gpt-6.2-sol');
    assert.equal(m.contextTokens, 1050000);
    assert.equal(m.maxOutputTokens, 128000);
    assert.deepEqual(m.capabilities.reasoning.effortLevels, ['low', 'medium', 'high', 'xhigh', 'max']);
    assert.equal(m.capabilities.reasoning.mode, 'always');
    assert.equal(m.nativeWebSearch.dialect, 'openAiResponses');
    for (const scope of ['us', 'global']) {
      const profile = provider('bedrock').models.find(model => model.id === `${scope}.openai.${id}`);
      assert.equal(profile.access, 'account_enablement');
      assert.equal(profile.contextTokens, m.contextTokens);
      assert.equal(profile.maxOutputTokens, m.maxOutputTokens);
      assert.equal(profile.nativeWebSearch, undefined);
    }
  }
  assert.equal(provider('openai').models.some(m => m.id.startsWith('gpt-6.2-') && m.id.endsWith('-pro')), false);
  for (const id of ['astra', 'astra-pro', 'sol', 'sol-pro']) {
    const m = provider('openrouter').models.find(m => m.id === `openai/gpt-6.2-${id}`);
    assert.equal(m.recommended, false);
    assert.deepEqual(m.capabilities.reasoning.effortLevels, ['low', 'medium', 'high', 'xhigh', 'max']);
    assert.ok(m.supportedParameters.includes('tools'));
    assert.ok(!m.supportedParameters.includes('temperature'));
  }
});
test('future aggregator expirations stay deprecated and endpoint-scoped', () => {
  for (const [id, date] of [['thinkingmachines/inkling-2-small:free', '2026-11-19'], ['mistralai/mistral-medium-3-5', '2027-01-31']]) {
    const m = provider('openrouter').models.find(m => m.id === id);
    assert.equal(m.status, 'deprecated'); assert.equal(m.deprecationDate, date);
  }
  for (const id of ['liquid/d1.1']) {
    const m = read('system-one-provider-presets').find(p => p.id === 'openrouter').models.find(m => m.id === id);
    assert.equal(m.status, 'deprecated'); assert.equal(m.deprecationDate, '2026-11-24');
  }
});
test('Alibaba Beijing lifecycles distinguish retired models from upcoming shutdowns', () => {
  const models = provider('alibaba-model-studio').models;
  for (const id of ['qwen3-max-2026-01-23', 'qwen3.6-max-preview', 'qwen3-max-preview', 'qwen3-coder-next', 'qwen3-coder-plus', 'qwen3-vl-flash']) {
    const model = models.find(model => model.id === id);
    assert.equal(model.status, 'removed');
    assert.equal(model.deprecationDate, '2026-10-10');
    assert.ok(model.replacementModelId);
  }
  const plus = models.find(model => model.id === 'qwen3.5-plus');
  assert.equal(plus.status, 'deprecated');
  assert.equal(plus.deprecationDate, '2027-01-19');
  assert.equal(plus.replacementModelId, 'qwen3.8-flash');
});
test('MAI image options and Decisions modality remain on their correct adapters', () => {
  for (const id of ['microsoft/mai-image-2.7', 'microsoft/mai-image-2.7-flash']) {
    const m = read('image-provider-presets').find(p => p.id === 'openrouter-images').models.find(m => m.id === id);
    assert.equal(m.supportedParameters.input_references.max, 5);
    assert.deepEqual(m.supportedParameters.background.values, ['opaque', 'transparent']);
    assert.equal(m.sizeOptions.filter(o => o.value === 'auto').length, 1);
  }
  const m = read('system-one-provider-presets').find(p => p.id === 'openrouter').models.find(m => m.id === 'microsoft/microsoft-decision-1.1');
  assert.deepEqual(m.inputModalities, ['text', 'image']);
  assert.deepEqual(m.outputModalities, ['decisions']);
});
test('Avatar 6 stays unselectable pending audio-driven operation contract', () => {
  for (const id of ['heygen/heygen-avatar-6', 'heygen/heygen-avatar-6-turbo']) {
    const m = read('video-provider-presets').find(p => p.id === 'openrouter-video').models.find(m => m.modelId === id);
    assert.equal(m.selectable, false); assert.equal(m.releaseStatus, 'contract_pending');
    assert.deepEqual(m.operationCapabilities, []);
  }
});
test('Macaron discovery remains Singapore/account-scoped and Doubao is not prematurely removed', () => {
  for (const id of ['mindlab/macaron-v1.1-venti', 'mindlab/macaron-v1.1-coding-venti', 'mindlab/macaron-v1-tall']) {
    const m = provider('qwen-cloud-intl').models.find(m => m.id === id);
    assert.deepEqual(m.regions, ['ap-southeast-1']);assert.equal(m.access, 'account_enablement');assert.equal(m.recommended, false);
    assert.equal(provider('alibaba-model-studio').models.some(m => m.id === id), false);
  }
  const m = provider('doubao').models.find(m => m.id === 'doubao-seed-2-0-pro-260215');
  assert.equal(m.status, 'deprecated');assert.equal(m.deprecationDate, '2026-11-24');
});
test('Runway Hailuo additions keep explicit media-free text controls and published schema limits', () => {
  const snapshot=JSON.parse(readFileSync(new URL('../../../crates/core/src/media_generation/adapters/fixtures/runway-video-contracts.json',import.meta.url),'utf8'));
  for(const id of ['hailuo3_highspeed','hailuo3_lite']) {
    const m=read('video-provider-presets').find(p=>p.id==='runway-global').models.find(m=>m.modelId===id);
    assert.deepEqual(m.operationCapabilities.map(o=>o.operation),['text_to_video']);
    const op=m.operationCapabilities[0],wire=op.wireContract;
    assert.deepEqual(op.inputRoles,[]);assert.equal(op.supportsAudio,false);assert.equal(op.supportsSeed,false);
    assert.equal(wire.promptMax,7000);assert.equal(wire.maxReferenceImages+wire.maxReferenceVideos+wire.maxReferenceAudio,0);
    const c=snapshot.contracts.find(c=>c.model===id&&c.path==='/v1/text_to_video');
    for(const o of op.durationOptions){assert.equal(o.minDurationSeconds,c.fields.duration.minimum);assert.equal(o.maxDurationSeconds,c.fields.duration.maximum);assert.ok(c.fields.resolution.enum.includes(o.resolution.toLowerCase()));}
    assert.equal(op.aspectRatios.includes('adaptive'),false);
  }
});
