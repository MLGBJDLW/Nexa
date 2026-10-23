import { test } from 'node:test';
import assert from 'node:assert/strict';
import { replaceRunUsage } from '../src/lib/liveUsageAggregate.ts';
import type { UsageSnapshot } from '../src/types/conversation.ts';
import { usageModelIdentity } from '../src/features/usage/modelIdentity.ts';

const run = (promptTokens: number, cacheReadTokens: number): UsageSnapshot => ({
  source: 'provider', promptTokens, cacheReadTokens, cacheMissTokens: promptTokens - cacheReadTokens,
  completionTokens: 10, totalTokens: promptTokens + 10, thinkingTokens: 0, cacheCreationTokens: 0,
  lastPromptTokens: promptTokens, providerRaw: { promptTokens, cacheReadTokens, cacheMissTokens: promptTokens - cacheReadTokens, completionTokens: 10, totalTokens: promptTokens + 10 },
});

test('live aggregate replaces the current run across repeated reads and reconnects', () => {
  const first = replaceRunUsage(null, 'first', run(100, 80));
  const started = replaceRunUsage(first, 'active', run(50, 20));
  const latest = replaceRunUsage(started, 'active', run(90, 30));
  assert.equal(latest.promptTokens, 190);
  assert.equal(latest.cacheReadTokens, 110);
  assert.equal(latest.lastPromptTokens, 90);
  assert.deepEqual(replaceRunUsage(latest, 'active', run(90, 30)), latest);
  assert.equal(replaceRunUsage(latest, 'next', run(10, 0)).promptTokens, 200);
});
test('cross-provider usage keeps exact versions and private model identities separate', () => {
  assert.equal(usageModelIdentity('anthropic/claude-sonnet-5.5'), usageModelIdentity('claude-sonnet-5-5'));
  assert.equal(usageModelIdentity('minimax/minimax-m3.1-flash'), usageModelIdentity('MiniMax-M3.1-Flash'));
  assert.equal(usageModelIdentity('mistralai/mistral-medium-3-5'), usageModelIdentity('mistral-medium-3-5'));
  assert.equal(usageModelIdentity('cohere/command-a-plus-05-2026'), usageModelIdentity('command-a-plus-05-2026'));
  assert.notEqual(usageModelIdentity('anthropic/claude-sonnet-5.5-20261001'), usageModelIdentity('claude-sonnet-5.5'));
  assert.notEqual(usageModelIdentity('minimax/minimax-m3.1-flash:free'), usageModelIdentity('MiniMax-M3.1-Flash'));
  assert.notEqual(usageModelIdentity('private-team/model'), usageModelIdentity('another-team/model'));
});
