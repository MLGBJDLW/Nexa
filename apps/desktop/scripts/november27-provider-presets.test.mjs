import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const readCatalog = file => JSON.parse(readFileSync(new URL(`../../../shared/${file}`, import.meta.url), 'utf8'));
const chat = readCatalog('provider-presets.json').find(preset => preset.id === 'openrouter');
const decision = readCatalog('system-one-provider-presets.json').find(preset => preset.id === 'openrouter');
const image = readCatalog('image-provider-presets.json').find(preset => preset.id === 'openrouter-images');
const speech = readCatalog('tts-provider-presets.json');

test('the exact new chat and decision IDs stay opt-in on their advertised OpenRouter operations', () => {
  const chatId = 'inclusionai/lingsante-3.1-air-vl';
  const decisionId = 'antdigital/lingdt-decision-fast-1.0:free';
  const vision = chat.models.find(row => row.id === chatId);
  assert.equal(vision?.recommended, false);
  assert.equal(vision?.contextTokens, 131072);
  assert.equal(vision?.maxOutputTokens, 32768);
  assert.equal(vision?.supportsTools, true);
  assert.equal(vision?.supportsStructuredOutput, true);
  assert.deepEqual(vision?.inputModalities, ['text', 'image']);
  assert.deepEqual(vision?.outputModalities, ['text']);
  assert.equal(vision?.capabilities?.reasoning, undefined);

  const classifier = decision.models.find(row => row.id === decisionId);
  assert.equal(classifier?.contextTokens, 32768);
  assert.equal(classifier?.recommended, false);
  assert.deepEqual(classifier?.inputModalities, ['text']);
  assert.deepEqual(classifier?.outputModalities, ['decisions']);
  assert(!chat.models.some(row => row.id === decisionId));
  assert(!decision.models.some(row => row.id === chatId));
  assert(!image.models.some(row => [chatId, decisionId].includes(row.id)));
  assert(!speech.some(preset => /openrouter/i.test(preset.id)));
});

test('OpenRouter GLM 4.6V keeps tools without advertising unsupported JSON output', () => {
  const model = chat.models.find(row => row.id === 'z-ai/glm-4.6v');
  assert.equal(model?.supportsTools, true);
  assert.equal(model?.supportsStructuredOutput, false);
  assert(model?.supportedParameters.includes('tools'));
  assert(!model?.supportedParameters.includes('structured_outputs'));
  assert(!model?.supportedParameters.includes('response_format'));
});
