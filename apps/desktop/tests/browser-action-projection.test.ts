import {
  emptyBrowserActionProjection, interruptBrowserActions, projectBrowserAction,
} from '../src/features/browser/actionProjection';

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const event = (callId: string, phase: string, action = 'click', tabId = 'tab-1', operationId = `op-${callId}`) => ({ callId, operationId, phase, action, tabId });

let entries = projectBrowserAction(emptyBrowserActionProjection(), event('one', 'moving'), 1);
entries = projectBrowserAction(entries, event('two', 'waiting', 'wait_for'), 2);
entries = projectBrowserAction(entries, event('one', 'committing'), 3);
entries = projectBrowserAction(entries, event('one', 'verified'), 4);
assert(entries.entries.length === 2, 'interleaved calls must retain one row per call');
assert(entries.entries[0].phase === 'verified', 'the original call receives its receipt');
assert(entries.entries[1].phase === 'waiting', 'an unrelated wait remains pending');
assert(projectBrowserAction(entries, event('one', 'moving'), 1) === entries, 'late progress must not revive completed work');
assert(projectBrowserAction(entries, event('one', 'failed'), 5) === entries, 'a terminal result is immutable');

entries = interruptBrowserActions(entries);
assert(entries.entries[0].phase === 'verified', 'takeover preserves completed evidence');
assert(entries.entries[1].phase === 'interrupted', 'takeover interrupts a live wait');
assert(projectBrowserAction(entries, event('two', 'verified', 'wait_for'), 5) === entries, 'stale completion must not hide a takeover');
assert(interruptBrowserActions(entries) === entries, 'repeated takeover should not cause another render');

let pending = projectBrowserAction(emptyBrowserActionProjection(), event('wait', 'waiting', 'wait_for'), 1);
pending = projectBrowserAction(pending, event('wait', 'observedPending', 'wait_for'), 2);
assert(pending.entries[0].phase === 'observedPending', 'an unmet condition is distinct from a failed action');
assert(projectBrowserAction(pending, { action: 'click', phase: 'moving', tabId: 'tab-1' }, 3) === pending, 'events without call identity are not projected');
assert(projectBrowserAction(pending, { callId: 'call_0', action: 'click', phase: 'moving', tabId: 'tab-1' }, 3) === pending, 'a raw provider call ID cannot replace host operation identity');
assert(projectBrowserAction(pending, event('bad', 'constructor'), 3) === pending, 'prototype names are not valid phases');
assert(projectBrowserAction(pending, event('bad', 'moving'), undefined) === pending, 'unsequenced events cannot revive evicted history');

let bounded = emptyBrowserActionProjection();
for (let index = 0; index < 50; index++) bounded = projectBrowserAction(bounded, event(String(index), 'verified'), index + 1);
assert(bounded.entries.length === 6 && bounded.entries[0].callId === '44', 'recent actions have bounded retention');
assert(projectBrowserAction(bounded, event('0', 'moving'), 1) === bounded, 'a late event cannot revive a terminal call after eviction');
assert(bounded.retiredThroughSequence === 44, 'a constant-size sequence watermark replaces per-call tombstones');
bounded = interruptBrowserActions(bounded, 60);
assert(projectBrowserAction(bounded, event('late-unseen', 'waiting', 'wait_for'), 55) === bounded, 'takeover revokes even progress that was not delivered yet');
bounded = projectBrowserAction(bounded, event('fresh', 'waiting', 'wait_for'), 61);
bounded = interruptBrowserActions(bounded, 59);
assert(bounded.entries[bounded.entries.length - 1]?.phase === 'waiting', 'an older takeover cannot interrupt a newer operation');

for (const action of ['click', 'wait_for']) {
  const phase = action === 'wait_for' ? 'waiting' : 'moving';
  let repeated = projectBrowserAction(emptyBrowserActionProjection(), event('call_0', phase, action, 'tab-1', 'operation-a'), 1);
  repeated = projectBrowserAction(repeated, event('call_0', 'verified', action, 'tab-1', 'operation-a'), 2);
  repeated = projectBrowserAction(repeated, event('call_0', phase, action, 'tab-1', 'operation-b'), 3);
  assert(repeated.entries.length === 2, 'provider call ID reuse must admit a distinct host invocation');
  assert(repeated.entries[0].phase === 'verified' && repeated.entries[1].phase === phase, 'prior terminal evidence must not suppress the new invocation');
  assert(projectBrowserAction(repeated, event('call_0', 'failed', action, 'tab-1', 'operation-a'), 4) === repeated, 'a late old-operation event must not fail the new invocation');
  repeated = projectBrowserAction(repeated, event('call_0', 'observedPending', action, 'tab-1', 'operation-b'), 5);
  assert(repeated.entries[1].phase === 'observedPending', 'the second invocation receives only its own outcome');
}
console.log('ok - browser action projection preserves scoped, bounded, monotonic evidence');
