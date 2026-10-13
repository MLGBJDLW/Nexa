import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
const { build } = createRequire(import.meta.resolve('vite'))('esbuild');
const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/lib/providerPresets.ts', import.meta.url))], bundle: true, platform: 'node', format: 'esm', write: false });
const { findProviderModelPreset: find, isRemovedProviderModel } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);
const read = name => JSON.parse(readFileSync(new URL(`../../../shared/${name}.json`, import.meta.url), 'utf8'));

test('October additions retain native endpoint identity and exact reasoning policy', () => {
  const meta = find({ provider: 'open_ai', baseUrl: 'https://api.meta.ai/v1', model: 'muse-wonder-1.0' });
  assert.equal(meta.descriptor.limits.contextTokens, 1048576);
  assert.equal(meta.descriptor.lifecycle, 'preview');
  assert.equal(meta.recommended, false);
  assert.equal(meta.capabilities.reasoning.mode, 'always');
  assert.equal(meta.capabilities.reasoning.defaultEffort, undefined);
  assert.deepEqual(meta.capabilities.reasoning.effortLevels, ['minimal', 'low', 'medium', 'high', 'xhigh', 'max']);
  assert.equal(find({ provider: 'open_ai', baseUrl: 'https://api.openai.com/v1', model: meta.id }), null);
  assert.equal(find({ provider: 'open_ai', baseUrl: 'https://tenant.example/v1', model: meta.id }), null);
  const router = find({ provider: 'openrouter', model: 'meta/muse-wonder' });
  assert.equal(router.descriptor.limits.maxOutputTokens, 943718);
  assert.equal(router.capabilities.reasoning.defaultEffort, 'medium');
  const step = find({ provider: 'openrouter', model: 'stepfun/step-5-preview' });
  assert.deepEqual(step.capabilities.reasoning.effortLevels, ['low', 'medium', 'high']);
  assert.equal(step.descriptor.limits.maxOutputTokens, 64000);
  assert.ok(find({ provider: 'openrouter', model: 'inclusionai/ling-3.0-flash-sante' }));
  assert.equal(isRemovedProviderModel('openrouter', 'inclusionai/ling-3.0-flash-sante:free'), true);
  assert.equal(isRemovedProviderModel('openrouter', 'google/gemini-2.5-pro-preview-05-06'), false);
});

test('announced future shutdowns remain selectable until their actual retirement', () => {
  const minimax = find({ provider: 'open_ai', baseUrl: 'https://api.minimax.io/v1', model: 'MiniMax-M2.7' });
  assert.equal(minimax.descriptor.lifecycle, 'deprecated');
  assert.equal(minimax.descriptor.deprecationDate, '2026-11-18');
  const images = read('image-provider-presets');
  for (const id of ['qwen-dashscope-cn','qwen-dashscope-intl']) {
    const row = images.find(p => p.id === id).models.find(m => m.id === 'qwen-image-2.1-turbo');
    assert.equal(row.status, 'active'); assert.equal(row.capabilities.imageEditing, false);
  }
  const image1 = images.find(p => p.id === 'openai').models.find(m => m.id === 'gpt-image-1');
  assert.equal(image1.status, 'deprecated'); assert.equal(image1.deprecationDate, '2026-10-23');
});

test('updated ACP packages remain version-pinned and preserve companion archive metadata', () => {
  const agents = read('external-agent-presets');
  assert.equal(agents.find(p => p.id === 'qwen-code').registryVersion, '0.25.1');
  assert.equal(agents.find(p => p.id === 'copilot-acp').registryVersion, '1.0.96');
  for (const agent of agents.filter(p => p.registryId)) {
    for (const distribution of [agent.distribution.npx, agent.distribution.uvx].filter(Boolean)) {
      assert.ok(distribution.package.includes(agent.registryVersion), `${agent.id} pins the audited version`);
    }
  }
  assert.ok(agents.find(p => p.id === 'acp-antigravity-acp').distribution.binary['windows-x86_64'].archive.endsWith('.zip'));
});
