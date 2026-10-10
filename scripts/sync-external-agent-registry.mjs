// Import a reviewed, saved official ACP registry. This never installs packages.
import fs from 'node:fs';
import crypto from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const input = process.argv[2];
if (!input) throw new Error('Usage: node scripts/sync-external-agent-registry.mjs <reviewed-registry.json>');
const raw = fs.readFileSync(input);
const registry = JSON.parse(raw);
if (registry.version !== '1.0.0' || !Array.isArray(registry.agents)) throw new Error('Unsupported ACP registry');
const output = path.join(root, 'shared/external-agent-presets.json');
const previous = JSON.parse(fs.readFileSync(output, 'utf8'));
const legacy = { gemini:'gemini_cli', opencode:'opencode', 'github-copilot-cli':'github_copilot_acp', 'claude-acp':'claude_code_acp', 'codex-acp':'codex_acp', 'qwen-code':'qwen_code', goose:'goose', auggie:'auggie' };
const seen = new Set();
const presets = registry.agents.map(agent => {
  if (!/^[a-z0-9][a-z0-9-]*$/.test(agent.id) || seen.has(agent.id) || !agent.version) throw new Error('Invalid/duplicate registry identity');
  seen.add(agent.id);
  for (const value of [agent.repository, agent.website].filter(Boolean)) {
    const url = new URL(value); if (url.protocol !== 'https:' || url.username || url.password) throw new Error('Invalid registry link');
  }
  const provider = legacy[agent.id] ?? `acp_${agent.id.replaceAll('-', '_')}`;
  const existing = previous.find(preset => preset.provider === provider);
  const distribution = agent.distribution;
  if (!distribution || !['npx','uvx','binary'].some(kind => distribution[kind])) throw new Error(`No launch contract for ${agent.id}`);
  let command, args, env;
  if (distribution.npx) {
    command = 'npx'; args = ['--yes',distribution.npx.package,...(distribution.npx.args ?? [])]; env=distribution.npx.env ?? {};
  } else if (distribution.uvx) {
    command = 'uvx'; args = [distribution.uvx.package,...(distribution.uvx.args ?? [])]; env=distribution.uvx.env ?? {};
  } else {
    const platform = distribution.binary['linux-x86_64'] ?? distribution.binary['darwin-aarch64'] ?? Object.values(distribution.binary)[0];
    command = platform.cmd.replaceAll('\\','/').split('/').at(-1); args=platform.args ?? []; env=platform.env ?? {};
  }
  if (existing && legacy[agent.id]) {
    // Preserve installed-CLI ownership, including native argv, across imports.
    command = existing.command; args = existing.args;
  }
  return { id:existing?.id ?? `acp-${agent.id}`,name:existing?.name ?? agent.name,provider,runtime:'acp',baseUrl:'',requiresApiKey:false,models:[],
    icon:existing?.icon ?? '◇',description:agent.description,command,args,env,docsUrl:agent.website ?? agent.repository,
    registryId:agent.id,registryVersion:agent.version,registrySource:'https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json',
    distribution };
});
presets.push(...previous.filter(preset => !preset.registryId && preset.provider === 'hermes'));
presets.push({id:'custom-acp',name:'Custom ACP agent',provider:'custom_acp',runtime:'acp',baseUrl:'',requiresApiKey:false,models:[],icon:'⌘',
  description:'Connect any installed ACP v1 agent using its executable, arguments and environment.',command:'',args:[],docsUrl:'https://agentclientprotocol.com/overview/agents'});
fs.writeFileSync(output, JSON.stringify(presets,null,2)+'\n');
console.log(`Imported ${registry.agents.length} published agents; payload SHA-256 ${crypto.createHash('sha256').update(raw).digest('hex')}`);
