import assert from 'node:assert/strict';
import { test } from 'node:test';
import { externalAgentProgress } from '../src/lib/externalAgentProgress.ts';
import type { TraceEvent } from '../src/lib/streaming/protocol.ts';

test('foreground progress and background compaction remain independent until their own completion', () => {
  const events: TraceEvent[] = [
    { id: '1', kind: 'status', code: 'external_agent_compacting', text: 'Compacting' },
    { id: '2', kind: 'status', code: 'external_agent_active', text: 'Active' },
    { id: '3', kind: 'reply', text: 'Working' },
    { id: '4', kind: 'status', code: 'external_agent_network_progress', text: 'Bytes received' },
  ];
  assert.equal(externalAgentProgress(events, true).compacting, true);
  assert.equal(externalAgentProgress(events, true).stage, 'active');
  events.push({ id: '5', kind: 'status', code: 'external_agent_compaction_complete', text: 'Done' });
  assert.equal(externalAgentProgress(events, true).compacting, false);
  assert.equal(externalAgentProgress(events, true).stage, 'active');
  assert.equal(externalAgentProgress(events.slice(0, 4), false).compacting, false);
});

test('Copilot network-only progress stays visible when the provider omits readable thoughts', () => {
  const events: TraceEvent[] = [
    { id: 'network', kind: 'status', code: 'external_agent_network_progress', text: 'Receiving' },
  ];
  assert.equal(externalAgentProgress(events, true).stage, 'receiving');
  events.push({ id: 'intent', kind: 'status', code: 'external_agent_activity', text: 'Checking project files' });
  events.push({ id: 'heartbeat', kind: 'status', code: 'external_agent_network_progress', text: 'Receiving' });
  assert.equal(externalAgentProgress(events, true).latest?.text, 'Checking project files');
  events.push({ id: 'opaque', kind: 'status', code: 'external_agent_reasoning_unavailable', text: 'Not shared' });
  assert.equal(externalAgentProgress(events, true).stage, 'activity');
  assert.equal(externalAgentProgress(events, true).reasoningUnavailable, true);
  events.push({ id: 'thought', kind: 'thinking', text: 'A readable summary' });
  assert.equal(externalAgentProgress(events, true).reasoningUnavailable, false);
});
