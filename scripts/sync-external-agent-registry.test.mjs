import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

test('registry refresh keeps reviewed runner constraints only for the same package pin', () => {
  const temporaryRoot = path.resolve(os.tmpdir());
  const root = fs.mkdtempSync(path.join(temporaryRoot, 'nexa-acp-registry-'));
  try {
    for (const directory of ['scripts', 'shared', 'apps/desktop/public/provider-icons/agents']) {
      fs.mkdirSync(path.join(root, directory), { recursive: true });
    }
    const script = path.join(root, 'scripts/sync-external-agent-registry.mjs');
    fs.copyFileSync(new URL('./sync-external-agent-registry.mjs', import.meta.url), script);
    fs.copyFileSync(new URL('../shared/retired-external-agents.json', import.meta.url), path.join(root, 'shared/retired-external-agents.json'));
    fs.writeFileSync(path.join(root, 'apps/desktop/public/provider-icons/agents/example.svg'), '<svg/>');
    const output = path.join(root, 'shared/external-agent-presets.json');
    const input = path.join(root, 'registry.json');
    const runnerArgs = ['--with', 'agent-client-protocol==0.8.1'];
    fs.writeFileSync(output, JSON.stringify([{ provider: 'acp_example', registryId: 'example',
      distribution: { uvx: { package: 'example==1.0.0', runnerArgs } } }]));
    for (const version of ['1.0.0', '2.0.0']) {
      fs.writeFileSync(input, JSON.stringify({ version: '1.0.0', agents: [{
        id: 'example', name: 'Example', version, repository: 'https://example.org/agent',
        distribution: { uvx: { package: `example==${version}`, args: ['acp'] } },
      }, { id: 'antigravity-acp', name: 'Suspended agent', version: '1.0.0', distribution: { npx: { package: 'agent@1.0.0' } } }] }));
      execFileSync(process.execPath, [script, input], { stdio: 'pipe' });
      const imported = JSON.parse(fs.readFileSync(output, 'utf8'))[0];
      assert.ok(!JSON.parse(fs.readFileSync(output, 'utf8')).some(agent => agent.registryId === 'antigravity-acp'));
      assert.deepEqual(imported.distribution.uvx.runnerArgs, version === '1.0.0' ? runnerArgs : undefined);
      assert.deepEqual(imported.args, [...(version === '1.0.0' ? runnerArgs : []), `example==${version}`, 'acp']);
    }
  } finally {
    assert(path.resolve(root).startsWith(temporaryRoot + path.sep));
    fs.rmSync(root, { recursive: true, force: true });
  }
});
