import assert from 'node:assert/strict';
import test from 'node:test';
import { codexAccountQuota, copilotAccountQuota, externalAccountQuota, quotaPercent } from '../src/lib/subscriptionUsage.ts';
import type { CodexAccountSnapshot, CopilotAccountSnapshot } from '../src/lib/api.ts';

test('account quota preserves unknown, zero, fractional and exhausted windows', () => {
  for (const value of [undefined, null, NaN, Infinity, '12']) assert.equal(quotaPercent(value), null);
  assert.equal(quotaPercent(0), 0);
  assert.equal(quotaPercent(.5), .5);
  assert.equal(quotaPercent(150), 100);
  const quota = codexAccountQuota({ available: true, account: { accountType: 'chatgpt', planType: 'pro' }, rateLimits: [
    { id: 'codex', primary: { usedPercent: .5, windowDurationMins: 300, resetsAt: 1791720000 }, secondary: null },
    { id: 'review', primary: { usedPercent: null, resetsAt: Number.MAX_VALUE }, secondary: { usedPercent: 100 } },
  ] } as unknown as CodexAccountSnapshot);
  assert.equal(quota.windows.length, 3);
  assert.equal(quota.windows[0].usedPercent, .5);
  assert.equal(quota.windows[0].durationMinutes, 300);
  assert.equal(quota.windows[1].usedPercent, null);
  assert.equal(quota.windows[1].resetsAt, null);
  assert.equal(quota.windows[2].usedPercent, 100);
});

test('Copilot remaining percentages have the opposite direction; unlimited is not an invented percentage', () => {
  const quota = copilotAccountQuota({ available: true, authenticated: true, quotas: [
    { id: 'premium', remainingPercent: 78.5, unlimited: false, resetDate: null },
    { id: 'chat', remainingPercent: 0, unlimited: true, resetDate: null },
    { id: 'unknown', remainingPercent: null, unlimited: false, resetDate: null },
  ] } as CopilotAccountSnapshot);
  assert.equal(quota.windows[0].usedPercent, 21.5);
  assert.equal(quota.windows[1].usedPercent, null);
  assert.equal(quota.windows[1].unlimited, true);
  assert.equal(quota.windows[2].usedPercent, null);
});

test('failed, signed-out and unsupported accounts never retain a successful quota', () => {
  assert.equal(codexAccountQuota({ available: false } as CodexAccountSnapshot).status, 'unavailable');
  assert.equal(codexAccountQuota({ available: true, account: null } as CodexAccountSnapshot).status, 'signedOut');
  assert.equal(codexAccountQuota({ available: true, account: { accountType: 'apiKey' }, rateLimits: [] } as unknown as CodexAccountSnapshot).status, 'unsupported');
  assert.deepEqual(externalAccountQuota({ status: 'unavailable', provider: 'anthropic', plan: null, fetchedAt: null,
    windows: [{ label: 'old', usedPercent: 0, resetsAt: null }], details: ['stale credit balance'] }).windows, []);
});
