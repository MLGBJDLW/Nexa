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
