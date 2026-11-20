import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
const presets = JSON.parse(readFileSync(new URL('../../../shared/provider-presets.json', import.meta.url), 'utf8'));
const models = id => presets.find(p => p.id === id).models;
test('OpenRouter withdrawal clears only the exact GLM route announcement', () => {
  const glm = models('openrouter').find(m => m.id === 'z-ai/glm-5.1');
  assert.equal(glm.status, 'active');
  assert.equal(glm.deprecationDate, undefined);
  assert.equal(models('doubao').find(m => m.id === 'glm-5-1-260428').deprecationDate, '2026-11-24');
  assert.equal(models('vertex').find(m => m.id === 'gemini-3.6-flash').status, 'deprecated');
});
test('Pixtral aggregator route does not invent tools or structured output', () => {
  const m = models('openrouter').find(m => m.id === 'mistralai/pixtral-large-2502');
  assert.deepEqual(m.inputModalities, ['text', 'image']);
  assert.equal(m.supportsTools, false);
  assert.equal(m.supportsStructuredOutput, false);
  assert.equal(m.capabilities.reasoning, null);
  assert.equal(m.recommended, false);
  assert.equal(m.maxOutputTokens, 102400);
});
test('Alibaba Prime remains explicit Beijing-only selection with base capabilities', () => {
  const base = models('alibaba-model-studio').find(m => m.id === 'kimi-k3.1-pro');
  const prime = models('alibaba-model-studio').find(m => m.id === 'kimi-k3.1-pro-prime');
  assert.deepEqual(prime.capabilities, base.capabilities);
  assert.deepEqual(prime.regions, ['cn-beijing']);
  assert.equal(prime.recommended, false);
  assert.equal(prime.access, 'account_enablement');
  for (const p of presets.filter(p => p.id !== 'alibaba-model-studio')) assert.ok(!p.models.some(m => m.id === prime.id));
});

test('other corroborated withdrawals stay scoped to their operation', () => {
  assert.equal(models('openrouter').find(m => m.id === 'z-ai/glm-4.7').deprecationDate, undefined);
  const decisions = JSON.parse(readFileSync(new URL('../../../shared/system-one-provider-presets.json', import.meta.url), 'utf8'));
  const m = decisions.find(p => p.id === 'openrouter').models.find(m => m.id === 'liquid/d1-ultra');
  assert.equal(m.status, 'active'); assert.equal(m.deprecationDate, undefined);
});
