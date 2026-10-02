import { mkdirSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

// Invoked only by the explicit manual live workflow step. Never print the
// configuration or API key; the provider receives its key through the environment.
const configuration = process.env.NEXA_EVAL_CONFIG;
if (!configuration) throw new Error('Configure the NEXA_EVAL_CONFIG repository variable before requesting live evaluation');
const config = JSON.parse(configuration);
if (!config.model || !config.baseUrl || !config.provider) throw new Error('Live evaluation requires explicit provider, baseUrl and model');
if (config.apiKey) throw new Error('Keep API keys in NEXA_EVAL_API_KEY, not in the configuration variable');
mkdirSync('target/eval', { recursive: true });
writeFileSync('target/eval/live-config.json', JSON.stringify(config), { mode: 0o600 });
const result = spawnSync('cargo', ['run', '-p', 'nexa-agent-eval', '--', '--live', '--config', 'target/eval/live-config.json', '--output', 'target/eval/live-report.json'], { stdio: 'inherit' });
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
