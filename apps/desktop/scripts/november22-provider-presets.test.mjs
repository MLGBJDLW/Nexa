import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = file => JSON.parse(readFileSync(new URL('../../../shared/' + file, import.meta.url), 'utf8'));
const presets = read('provider-presets.json');
const find = (provider, id) => presets.find(p => p.id === provider).models.find(m => m.id === id);

test('Inkling 2 Mini is an exact opt-in OpenRouter route with its hosted limits', () => {
  const id = 'thinkingmachines/inkling-2-mini';
  const row = find('openrouter', id);
  assert.equal(row.status, 'active');
  assert.equal(row.recommended, false);
  assert.deepEqual(row.inputModalities, ['text', 'image']);
  assert.deepEqual(row.outputModalities, ['text']);
  assert.equal(row.contextTokens, 1_048_576);
  assert.equal(row.maxOutputTokens, 65_536);
  assert.equal(row.supportsTools, true);
  assert.equal(row.supportsStructuredOutput, true);
  assert.equal(row.capabilities.reasoning.mode, 'optional');
  assert.equal(row.capabilities.reasoning.defaultEffort, 'high');
  assert.equal(row.capabilities.reasoning.thinkingBudget.enabled, false);
  assert.ok(presets.filter(p => p.id !== 'openrouter').every(p => !p.models.some(m => m.id === id)));
});

test('DaisyRose reasoning only exposes the exact newly advertised effort set', () => {
  const reasoning = find('openrouter', 'radixark/daisyrose-27b').capabilities.reasoning;
  assert.equal(reasoning.mode, 'optional');
  assert.deepEqual(reasoning.effortLevels, ['none', 'low', 'medium', 'xhigh']);
  assert.equal(reasoning.defaultEffort, 'xhigh');
  assert.equal(reasoning.thinkingBudget.enabled, false);
});

test('OpenRouter MiniMax M2.7 does not advertise unsupported structured output', () => {
  const row = find('openrouter', 'minimax/minimax-m2.7');
  assert.equal(row.maxOutputTokens, 131_072);
  assert.equal(row.supportsTools, true);
  assert.equal(row.supportsStructuredOutput, false);
  assert.ok(!row.supportedParameters.includes('response_format'));
  assert.ok(!row.supportedParameters.includes('structured_outputs'));
});

test('native and OpenRouter Grok Build retirements retain independent evidence', () => {
  const row = find('xai', 'grok-build-0.1');
  assert.equal(row.status, 'removed');
  assert.equal(row.deprecationDate, '2026-11-19');
  assert.equal(row.replacementModelId, undefined);
  assert.equal(find('xai', 'grok-code-fast-1').replacementModelId, undefined);
  const gateway = find('openrouter', 'x-ai/grok-build-0.1');
  assert.equal(gateway.status, 'removed');
  assert.equal(gateway.documentationRef, 'https://openrouter.ai/api/v1/models/x-ai/grok-build-0.1/endpoints');
  assert.equal(gateway.replacementModelId, undefined);
});

test('Liquid D1 variants have scoped retirement and opt-in stable guidance', () => {
  const models = read('system-one-provider-presets.json').find(p => p.id === 'openrouter').models;
  const row = id => models.find(m => m.id === id);
  assert.equal(row('liquid/d1').status, 'removed');
  assert.equal(row('liquid/d1').deprecationDate, '2026-11-20');
  assert.equal(row('liquid/d1.1').status, 'removed');
  assert.equal(row('liquid/d1.1').deprecationDate, '2026-11-24');
  assert.equal(row('liquid/d1').replacementModelId, 'liquid/d1.2');
  assert.equal(row('liquid/d1.1').replacementModelId, 'liquid/d1.2');
  assert.equal(row('liquid/d1.2').status, 'active');
  assert.equal(row('liquid/d1-ultra').status, 'active');
  assert.equal(row('liquid/d1-ultra').deprecationDate, undefined);
  assert.equal(row('~liquid/d1-latest').status, 'active');
});
