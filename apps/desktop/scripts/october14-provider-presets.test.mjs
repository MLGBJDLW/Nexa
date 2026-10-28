import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
const read = name => JSON.parse(readFileSync(new URL(`../../../shared/${name}.json`, import.meta.url), 'utf8'));
const providers = read('provider-presets');
const model = (provider, id) => providers.find(item => item.id === provider).models.find(item => item.id === id);

test('October 28 Terra routes retain different reasoning controls and no automatic selection', () => {
  for (const [provider, id, mandatory] of [
    ['openai', 'gpt-6.1-terra', false],
    ['openrouter', 'openai/gpt-6.1-terra', false],
    ['openrouter', 'openai/gpt-6.1-terra-pro', true],
  ]) {
    const row = model(provider, id);
    assert.equal(row.contextTokens, 1_050_000);
    assert.equal(row.maxOutputTokens, 128_000);
    assert.equal(row.supportsTools, true);
    assert.equal(row.capabilities.vision, true);
    assert.equal(row.capabilities.reasoning.mode, mandatory ? 'always' : 'optional');
    assert.deepEqual(row.capabilities.reasoning.effortLevels, mandatory
      ? ['low', 'medium', 'high', 'xhigh', 'max']
      : ['none', 'low', 'medium', 'high', 'xhigh', 'max']);
    assert.equal(row.productReadiness, 'known');
    assert.equal(row.recommended, false);
  }
  const bonsai = model('openrouter', 'prism-ml/ternary-bonsai-2-9b');
  assert.equal(bonsai.contextTokens, 262_144);
  assert.equal(bonsai.maxOutputTokens, 65_536);
  assert.deepEqual(bonsai.capabilities.reasoning.effortLevels, []);
  assert.equal(bonsai.capabilities.reasoning.thinkingBudget.enabled, false);
  assert.equal(bonsai.supportedParameters.includes('reasoning_effort'), false);
  const flash = model('siliconflow', 'zai-org/GLM-5.3-Flash');
  assert.equal(flash.capabilities.vision, true);
  assert.equal(flash.capabilities.reasoning, null);
  assert.equal(model('zhipu-intl', 'glm-5.3-flashx-legal').access, 'public');
  assert.equal(model('zhipu', 'glm-5.3-flashx-legal').access, 'application');
});

test('October 23 shutdowns retire only the verified endpoints without paid replacements', () => {
  for (const id of ['gpt-4.1-nano', 'o4-mini', 'o3-mini']) {
    assert.equal(model('openai', id).status, 'removed');
    assert.equal(model('openai', id).deprecationDate, '2026-10-23');
  }
  const images = read('image-provider-presets').find(row => row.id === 'openai').models;
  assert.equal(images.find(row => row.id === 'gpt-image-1').status, 'removed');
  assert.equal(images.find(row => row.id === 'gpt-image-1-mini').status, 'deprecated');
  for (const id of ['arcee-ai/trinity-large-thinking', 'ibm-granite/granite-4.0-h-micro', 'nvidia/nemotron-3-super-120b-a12b:free']) {
    assert.equal(model('openrouter', id).status, 'removed');
    assert.equal(model('openrouter', id).replacementModelId, undefined);
  }
  assert.notEqual(model('openrouter', 'openai/gpt-4.1-nano').status, 'removed');
});

test('Titan embeddings expose only supported text dimensions and canonical decision models', () => {
  const embeddings = read('embedding-provider-presets').find(row => row.id === 'openrouter').models;
  for (const [id, dimensions, context, allowed] of [
    ['amazon/titan-embed-text-v1', 1536, 8192, []],
    ['amazon/titan-embed-text-v2', 1024, 8192, [1024, 512, 256]],
  ]) {
    const row = embeddings.find(row => row.id === id);
    assert.equal(row.dimensions, dimensions);
    assert.equal(row.contextTokens, context);
    assert.deepEqual(row.inputModalities, ['text']);
    assert.equal(row.supportsDimensionOverride, allowed.length > 0);
    assert.deepEqual(row.allowedDimensions ?? [], allowed);
  }
  const decisions = read('system-one-provider-presets').find(row => row.id === 'openrouter').models;
  assert.equal(embeddings.some(row => row.id === 'amazon/titan-embed-image-v1'), false,
    'the 128-token multimodal model needs a dedicated input budget, not tiny general-index chunks');
  assert.equal(decisions.find(row => row.id === 'liquid/d1.1').contextTokens, 65_536);
  assert.equal(decisions.some(row => row.id === '~liquid/d1-latest'), false);
});

test('new embedding routes preserve exact dimensions and text adapter boundaries', () => {
  const embeddings = read('embedding-provider-presets');
  const lookup = (provider, id) => embeddings.find(item => item.id === provider).models.find(item => item.id === id);
  for (const [id, dimensions, context] of [['small', 1024, 32768], ['nano', 768, 8192]]) {
    const row = lookup('jina', `jina-embeddings-v5-omni-${id}`);
    assert.equal(row.dimensions, dimensions);
    assert.equal(row.contextTokens, context);
    assert.deepEqual(row.inputModalities, ['text']);
    assert.equal(row.minDimensions, 32);
    assert.equal(row.maxDimensions, dimensions);
  }
  for (const [id, dimensions] of [['voyageai/voyage-4', 1024], ['voyageai/voyage-4-large', 1024], ['voyageai/voyage-4-lite', 1024], ['voyageai/voyage-code-4', 1024], ['qwen/qwen3-embedding-4b', 2560], ['mistralai/mistral-embed-2312', 1024], ['mistralai/codestral-embed-2505', 1536]]) {
    const row = lookup('openrouter', id);
    assert.equal(row.dimensions, dimensions);
    assert.equal(row.supportsDimensionOverride, false);
    assert.equal(row.status, 'active');
  }
});

test('new native model IDs carry endpoint-specific limits and reasoning defaults', () => {
  const hostedMiniMax = model('alibaba-model-studio', 'MiniMax/MiniMax-M3.1-Flash');
  assert.equal(hostedMiniMax.capabilities.reasoning.mode, 'always');
  assert.deepEqual(hostedMiniMax.capabilities.reasoning.effortLevels, ['low', 'medium', 'high', 'xhigh', 'max']);
  assert.equal(model('zhipu', 'glm-5.3-flashx-legal').access, 'application');
  assert.equal(model('siliconflow', 'deepseek-ai/DeepSeek-V4.1-Flash').capabilities.reasoning, null);
  assert.equal(model('openrouter', 'recursive/flint-v1.0:free').capabilities.reasoning.mode, 'always');
  assert.equal(model('alibaba-model-studio', 'deepseek-v4.1-flash-1017').contextTokens, 1000000);
  assert.deepEqual(model('qwen-cloud-intl', 'deepseek-v4.1-flash-1017').regions, ['ap-southeast-1']);
  const argon = model('google', 'gemini-4-argon');
  assert.equal(argon.contextTokens, 1_048_576);
  assert.equal(argon.maxOutputTokens, 65_536);
  assert.equal(argon.capabilities.promptCache, true, 'Argon supports implicit cache reuse; explicit cache markers remain disabled by its runtime profile');
  assert.deepEqual(argon.capabilities.reasoning.effortLevels, ['low', 'medium', 'high']);
  for (const provider of ['google', 'vertex']) {
    for (const id of ['gemini-3.6-flash', 'gemini-3.7-flash', 'gemini-3.8-flash']) {
      assert.equal(model(provider, id).capabilities.reasoning.mode, 'always', `${provider}/${id} cannot disable thinking`);
    }
  }
  assert.equal(model('doubao', 'glm-5-2-260617').capabilities.reasoning.defaultEffort, 'high');
  assert.deepEqual(model('doubao', 'glm-5-2-260617').capabilities.reasoning.effortLevels, ['high', 'max']);
  for (const id of ['doubao-seed-2-0-mini-260428', 'doubao-seed-character-260628']) assert.equal(model('doubao', id).capabilities.reasoning.defaultEffort, 'medium');
  for (const id of ['deepseek-v4-pro-ga-260813', 'deepseek-v4-flash-ga-260731']) assert.equal(model('doubao', id).maxOutputTokens, 384_000);
  assert.equal(model('openrouter', 'liquid/lfm-2.5-24b-a2b').contextTokens, 128_000);
  assert.equal(model('openrouter', 'liquid/lfm-2.5-2.6b').contextTokens, 128_000);
  assert.equal(model('openrouter', 'liquid/lfm-2.5-2.6b:free').contextTokens, 65_536);
  assert.equal(model('openrouter', 'liquid/lfm-2.5-2.6b:free').replacementModelId, undefined, 'free selections must not silently switch to a paid route');
  const arkKimi = model('doubao', 'kimi-k3-1-pro-261024');
  assert.equal(arkKimi.contextTokens, 1_024_000);
  assert.equal(arkKimi.maxOutputTokens, 128_000);
  assert.deepEqual(arkKimi.capabilities.reasoning.effortLevels, ['low', 'high', 'max']);
  assert.equal(arkKimi.capabilities.reasoning.mode, 'always');
  for (const id of ['scaledown/summarize', 'scaledown/extract']) {
    assert.equal(model('openrouter', id).supportsTools, false);
    assert.equal(model('openrouter', id).recommended, false);
  }
});

test('confirmed route removals remain tombstones while future retirements remain deprecated', () => {
  for (const id of ['openai/gpt-3.5-turbo', 'openai/gpt-3.5-turbo-instruct']) {
    assert.equal(model('openrouter', id).status, 'removed');
    assert.equal(model('openrouter', id).replacementModelId, undefined);
  }
  assert.equal(model('openrouter', 'minimax/minimax-m3.1-flash:free').status, 'removed');
  assert.equal(model('openrouter', 'minimax/minimax-m3.1-flash:free').replacementModelId, undefined, 'an unavailable free route must never migrate into billing');
  assert.equal(model('openrouter', 'minimax/minimax-m3.1-flash').status, 'active');
  assert.equal(model('alibaba-model-studio', 'xiaomi/mimo-v2.5-pro').status, 'removed');
  assert.equal(model('alibaba-model-studio', 'xiaomi/mimo-v2.5-pro').replacementModelId, 'xiaomi/mimo-v2.6-pro-ultraspeed');
  for (const id of ['apodex/apodex-1.1-mini:free', 'inclusionai/ling-3.0-flash-fin', 'inclusionai/ling-3.0-flash-sante']) assert.equal(model('openrouter', id).status, 'removed');
  assert.equal(model('openrouter', 'google/gemini-2.5-pro-preview-05-06').status, 'removed');
  for (const id of ['gpt-4o', 'gpt-4o-mini', 'gpt-4.1', 'gpt-4.1-mini']) {
    assert.equal(model('openai', id).status, 'deprecated');
    assert.equal(model('openai', id).deprecationDate, '2027-01-14');
  }
  assert.equal(read('image-provider-presets').flatMap(item => item.models).find(item => item.id === 'inclusionai/ming-image-0.1-design').status, 'removed');
});

test('routed aliases retain compatibility and new embedding routes use supported text contracts', () => {
  for (const [id, replacement] of [['gemini-3.7-flash', 'gemini-3.8-flash'], ['gemini-3.5-flash', 'gemini-3.6-flash']]) {
    assert.equal(model('google', id).status, 'deprecated');
    assert.equal(model('google', id).replacementModelId, replacement);
  }
  assert.equal(model('openrouter', 'unbiased/pareto').replacementModelId, '~unbiased/pareto-latest');
  const embeddings = read('embedding-provider-presets').find(row => row.id === 'openrouter').models;
  for (const [id, dimensions, context] of [
    ['liquid/lfm-2.5-embedding-350m:free', 1024, 512],
    ['nvidia/nemotron-3-embed-1b:free', 2048, 32768],
    ['google/gemini-embedding-2', 3072, 8192],
    ['voyageai/voyage-multimodal-3.5', 1024, 32000],
  ]) {
    const row = embeddings.find(row => row.id === id);
    assert.equal(row.dimensions, dimensions);
    assert.equal(row.contextTokens, context);
    assert.deepEqual(row.inputModalities, ['text']);
    assert.equal(row.supportsDimensionOverride, false);
  }
});
