import assert from 'node:assert/strict';
import test from 'node:test';
import { isLocalWorkspaceArtifact } from './repository-policy.mjs';

test('rejects agent context files at every directory depth and transient patch payloads', () => {
  for (const file of ['CONTEXT.md', 'apps/desktop/AGENTS.md', 'CLAUDE.md', 'Gemini.md',
    '.codex/plan.md', '.agents/context.json', '.claude/settings.json', '.github/nexa-patches/chunk-00.b64']) {
    assert.equal(isLocalWorkspaceArtifact(file), true, file);
  }
});

test('rejects the existing local research and investigation locations', () => {
  for (const file of ['docs/local/check.md', 'docs/research/sources.md', 'docs/architecture/old.md',
    'docs/runtime-primary-source-research.md', 'docs/runtime/browser-implementation-research.md']) {
    assert.equal(isLocalWorkspaceArtifact(file), true, file);
  }
});

test('keeps maintained documentation, product prompts, packaged skills and test fixtures', () => {
  for (const file of ['docs/ARCHITECTURE.md', 'docs/SUBSCRIPTION_AGENTS.md',
    'crates/core/prompts/system.md', 'crates/core/assets/skills/skill-creator/SKILL.md',
    'crates/core/assets/skills/research-synthesis/references/research-patterns.md',
    'testdata/sample_vault/notes/project-ideas.md', '.github/workflows/ci.yml']) {
    assert.equal(isLocalWorkspaceArtifact(file), false, file);
  }
});
