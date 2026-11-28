import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = name => JSON.parse(readFileSync(new URL('../../../shared/' + name + '-provider-presets.json', import.meta.url), 'utf8'));
const presets = JSON.parse(readFileSync(new URL('../../../shared/provider-presets.json', import.meta.url), 'utf8'));
const model = (provider, id) => presets.find(p => p.id === provider).models.find(m => m.id === id);

test('Ark announces same-route migration guidance without removing saved models', () => {
  for (const [id, deadline, replacement] of [
    ['doubao-seed-2-0-pro-260215', '2027-01-22', 'doubao-seed-2-1-lite-260915'],
    ['doubao-seed-2-0-code-preview-260215', '2026-11-24', 'doubao-seed-evolving'],
    ['glm-5-1-260428', '2026-11-24', 'doubao-seed-evolving'],
    ['deepseek-v4-pro-260425', '2026-11-24', 'deepseek-v4-1-flash-261017'],
  ]) {
    const row = model('doubao', id);
    assert.equal(row.status, deadline === '2026-11-24' ? 'removed' : 'deprecated');
    assert.equal(row.deprecationDate, deadline);
    assert.equal(row.replacementModelId, replacement);
    assert.equal(model('doubao', replacement).status, 'active');
  }
  assert.equal(model('openrouter', 'z-ai/glm-5.1').status, 'active');
  assert.equal(model('openrouter', 'z-ai/glm-5.1').replacementModelId, undefined);
});

test('Vertex extension and replacement do not alter Gemini API or OpenRouter', () => {
  for (const id of ['gemini-3.6-flash', 'gemini-3.7-flash']) {
    const row = model('vertex', id);
    assert.equal(row.status, 'deprecated');
    assert.equal(row.deprecationDate, '2027-01-28');
    assert.equal(row.replacementModelId, 'gemini-4-hydrogen');
    assert.equal(model('openrouter', 'google/' + id).status, 'active');
  }
  assert.equal(model('google', 'gemini-3.6-flash').status, 'active');
  assert.equal(model('google', 'gemini-3.7-flash').replacementModelId, 'gemini-3.8-flash');
});

test('DaisyRose hosted boundaries remain distinct from other aggregator and open-weight routes', () => {
  const row = model('openrouter', 'radixark/daisyrose-27b');
  assert.deepEqual(row.inputModalities, ['text', 'image']);
  assert.deepEqual(row.outputModalities, ['text']);
  assert.equal(row.contextTokens, 262_144);
  assert.equal(row.maxOutputTokens, 235_929);
  assert.equal(row.supportsTools, true);
  assert.equal(row.supportsStructuredOutput, true);
  assert.equal(row.recommended, false);
  assert.equal(model('openrouter', 'xiaomi/mimo-v2.6-flash').contextTokens, 1_048_576);
  assert.equal(model('openrouter', 'xiaomi/mimo-v2.6-pro').contextTokens, 1_048_576);
  assert.equal(model('openrouter', 'deepseek/deepseek-v4-flash-vision-exp').maxOutputTokens, 262_144);
});

test('Macaron Venti Pro is an opt-in Singapore cloud route, not a token-plan model', () => {
  const id = 'mindlab/macaron-v1.1-venti-pro';
  const row = model('qwen-cloud-intl', id);
  assert.deepEqual(row.regions, ['ap-southeast-1']);
  assert.deepEqual(row.inputModalities, ['text', 'image']);
  assert.equal(row.contextTokens, 1_048_576);
  assert.equal(row.maxOutputTokens, 131_072);
  assert.equal(row.access, 'account_enablement');
  assert.equal(row.recommended, false);
  assert.equal(row.supportsTools, true);
  assert.equal(row.supportsStructuredOutput, true);
  assert.ok(presets.filter(p => p.id !== 'qwen-cloud-intl').every(p => !p.models.some(m => m.id === id)));
});

test('Baseten rows are explicit non-default decision selects, not chat models', () => {
  const decisions = read('system-one').find(p => p.id === 'openrouter').models;
  for (const [id, limit] of [['baseten/groundhog-1', 16384], ['baseten/bison-1', 32768]]) {
    const row = decisions.find(m => m.id === id);
    assert.deepEqual(row.inputModalities, ['text', 'image']);
    assert.deepEqual(row.outputModalities, ['decisions']);
    assert.equal(row.contextTokens, limit);
    assert.equal(row.recommended, false);
    assert.equal(model('openrouter', id), undefined);
  }
});
