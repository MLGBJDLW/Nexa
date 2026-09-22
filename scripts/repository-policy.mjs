import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Product prompts and bundled skill assets are source files. Local agent
// instructions and one-off investigations are workspace state instead.
export function isLocalWorkspaceArtifact(file) {
  return /(?:^|\/)(?:AGENTS|CLAUDE|CONTEXT|GEMINI)\.md$/iu.test(file)
    || /^(?:\.codex|\.agents|\.claude|\.github\/nexa-patches)\//u.test(file)
    || /^docs\/(?:local|research|architecture)\//u.test(file)
    || /^docs\/.*-(?:primary-source|implementation)-research\.md$/u.test(file);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL('../', import.meta.url));
  const tracked = execFileSync('git', ['ls-files', '-z'], { cwd: root, encoding: 'utf8' })
    .split('\0').filter(Boolean).filter(isLocalWorkspaceArtifact);
  if (tracked.length) {
    console.error(`Local workspace artifacts must not be tracked:\n${tracked.join('\n')}`);
    process.exitCode = 1;
  } else {
    console.log('Repository policy passed: no tracked local workspace artifacts.');
  }
}
