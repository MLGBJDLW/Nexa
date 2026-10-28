import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { mkdtempSync, mkdirSync, copyFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
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
  assert.equal(isRemovedProviderModel('openrouter', 'inclusionai/ling-3.0-flash-sante'), true);
  assert.equal(isRemovedProviderModel('openrouter', 'inclusionai/ling-3.0-flash-sante:free'), true);
  assert.equal(isRemovedProviderModel('openrouter', 'google/gemini-2.5-pro-preview-05-06'), true);
});

test('retired routes are removed while future shutdowns remain deprecated', () => {
  const minimax = find({ provider: 'open_ai', baseUrl: 'https://api.minimax.io/v1', model: 'MiniMax-M2.7' });
  assert.equal(minimax.descriptor.lifecycle, 'deprecated');
  assert.equal(minimax.descriptor.deprecationDate, '2026-11-18');
  const images = read('image-provider-presets');
  for (const id of ['qwen-dashscope-cn','qwen-dashscope-intl']) {
    const row = images.find(p => p.id === id).models.find(m => m.id === 'qwen-image-2.1-turbo');
    assert.equal(row.status, 'active'); assert.equal(row.capabilities.imageEditing, false);
  }
  const image1 = images.find(p => p.id === 'openai').models.find(m => m.id === 'gpt-image-1');
  assert.equal(image1.status, 'removed'); assert.equal(image1.deprecationDate, '2026-10-23');
});

test('updated ACP packages remain version-pinned and preserve companion archive metadata', () => {
  const agents = read('external-agent-presets');
  assert.equal(agents.find(p => p.id === 'qwen-code').registryVersion, '0.25.1');
  assert.equal(agents.find(p => p.id === 'copilot-acp').registryVersion, '1.0.98');
  for (const agent of agents.filter(p => p.registryId)) {
    for (const distribution of [agent.distribution.npx, agent.distribution.uvx].filter(Boolean)) {
      assert.ok(distribution.package.includes(agent.registryVersion), `${agent.id} pins the audited version`);
    }
    for (const binary of Object.values(agent.distribution.binary ?? {})) {
      if (binary.sha256) assert.match(binary.sha256, /^[a-f0-9]{64}$/i, `${agent.id} preserves its publisher digest`);
      else assert.equal(agent.registrySource, 'https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json', `${agent.id} unpinned archive needs official registry provenance`);
    }
  }
  assert.ok(!agents.some(p => p.registryId === 'antigravity-acp'));
});

test('registry refresh keeps reviewed archive pins without carrying them to a new artifact', () => {
  const root = mkdtempSync(path.join(tmpdir(), 'nexa-registry-pins-'));
  try {
    for (const directory of ['scripts', 'shared', 'apps/desktop/public/provider-icons/agents']) {
      mkdirSync(path.join(root, directory), { recursive: true });
    }
    const script = path.join(root, 'scripts/sync-external-agent-registry.mjs');
    copyFileSync(new URL('../../../scripts/sync-external-agent-registry.mjs', import.meta.url), script);
    copyFileSync(new URL('../../../shared/retired-external-agents.json', import.meta.url), path.join(root, 'shared/retired-external-agents.json'));
    const original = { archive: 'https://publisher.example/v1/agent.zip', cmd: 'agent', sha256: 'a'.repeat(64) };
    writeFileSync(path.join(root, 'shared/external-agent-presets.json'), JSON.stringify([
      { provider: 'acp_fixture', distribution: { binary: { same: original, changed: original } } },
    ]));
    writeFileSync(path.join(root, 'apps/desktop/public/provider-icons/agents/fixture.svg'), '<svg/>');
    const input = path.join(root, 'registry.json');
    writeFileSync(input, JSON.stringify({ version: '1.0.0', agents: [{
      id: 'fixture', version: '1', name: 'Fixture', repository: 'https://publisher.example',
      distribution: { binary: {
        same: { archive: original.archive, cmd: original.cmd },
        changed: { archive: 'https://publisher.example/v2/agent.zip', cmd: original.cmd },
      } },
    }] }));
    execFileSync(process.execPath, [script, input], { stdio: 'pipe' });
    const [agent] = JSON.parse(readFileSync(path.join(root, 'shared/external-agent-presets.json'), 'utf8'));
    assert.equal(agent.distribution.binary.same.sha256, original.sha256);
    assert.equal(agent.distribution.binary.changed.sha256, undefined);
  } finally {
    assert.equal(path.dirname(path.resolve(root)), path.resolve(tmpdir()));
    assert.ok(path.basename(root).startsWith('nexa-registry-pins-'));
    rmSync(root, { recursive: true, force: true });
  }
});
