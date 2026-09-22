import { extractManagedProcess, mergeToolActivityEvents } from '../src/lib/processArtifacts';
import { applyToolRunEvent } from '../src/lib/streaming/toolProjection';
import { createDefaultState } from '../src/lib/streaming/state';
import type { ActivityEvent, ToolRunItem } from '../src/types/conversation';

function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
function event(seq: number, kind: ActivityEvent['kind'], payload: Record<string, unknown>): ActivityEvent {
  return { activityId: 'server', seq, kind, timestamp: '2026-09-22T00:00:00Z', payload };
}
const run = { callId: 'start-server', toolName: 'run_shell', status: 'running' } as ToolRunItem;
const state = createDefaultState();
applyToolRunEvent(state, { ...run, artifacts: { activity: event(1, 'started', { state: 'running' }) } });
applyToolRunEvent(state, { ...run, artifacts: { activity: event(2, 'stdout_chunk', { data: 'Starting\n' }) } });
applyToolRunEvent(state, { ...run, artifacts: { activity: event(3, 'stderr_chunk', { data: 'Compiling\n' }) } });
applyToolRunEvent(state, { ...run, progressNote: 'heartbeat' });
const call = state.toolCalls[0];
assert(call.activityEvents?.length === 3, 'progress must accumulate and survive a heartbeat');
assert(extractManagedProcess(call.toolName, call.artifacts, call.activityEvents)?.output === 'Starting\nCompiling\n', 'both streams must reach the visible output');

const ready = { kind: 'managedService', activityId: 'server', cursor: 4, status: 'ready', readyUrl: 'http://127.0.0.1:5173/', stdoutTail: 'Ready\n' };
applyToolRunEvent(state, { ...run, status: 'completed', artifacts: ready });
const completed = state.toolCalls[0];
assert(completed.activityEvents?.length === 3, 'completion must retain activity history');
assert(extractManagedProcess('run_shell', completed.artifacts, completed.activityEvents)?.output === 'Ready\n', 'receipt tail must not duplicate streamed logs');
assert(extractManagedProcess('run_shell', ready)?.readyUrl === 'http://127.0.0.1:5173/', 'verified service can open its URL');
for (const url of ['javascript:alert(1)', 'https://example.com/', 'http://user:secret@localhost:5173/']) {
  assert(extractManagedProcess('run_shell', { ...ready, readyUrl: url })?.readyUrl === null, 'only credential-free loopback receipts may open');
}
assert(extractManagedProcess('run_shell', { ...ready, status: 'unhealthy' })?.readyUrl === null, 'unhealthy endpoints must not be presented as ready');
assert(extractManagedProcess('run_shell', { ...ready, status: 'exited', execution: { exitCode: 1 } })?.state === 'failed', 'a failed build is not successful completion');
assert(extractManagedProcess('run_shell', { ...ready, status: 'exited', execution: { exitCode: 0 } })?.state === 'completed', 'a zero exit code completes the process');

const observation = { kind: 'activityObservation', service: ready, activity: { record: { activityId: 'server', ownerTool: 'run_shell', state: 'ready' }, cursor: 4, events: [event(4, 'ready_url', { url: ready.readyUrl })] } };
assert(extractManagedProcess('activity_observe', observation)?.readyUrl === ready.readyUrl, 'delayed readiness must project nested service receipts');
const stopped = mergeToolActivityEvents(completed.activityEvents, { activity: event(5, 'cancelled', { state: 'cancelled' }) });
assert(extractManagedProcess('run_shell', ready, stopped)?.readyUrl === null, 'newer termination must revoke the stale ready button');
for (const kind of ['timed_out', 'superseded'] as const) {
  assert(extractManagedProcess('run_shell', ready, [event(5, kind, {})])?.readyUrl === null, 'every terminal event revokes stale ready URLs');
}
assert(extractManagedProcess('activity_observe', { kind: 'activityObservation', activity: { record: { activityId: 'server', ownerTool: 'run_shell', surface: 'process', state: 'orphaned' }, cursor: 4, events: [] } })?.state === 'unavailable', 'recovered orphaned processes are not advertised as running');
assert(extractManagedProcess('activity_observe', { kind: 'activityObservation', activity: { record: { activityId: 'server', ownerTool: 'browser_session', surface: 'browser' }, events: [event(1, 'started', {})] } }) === null, 'browser activities do not become process cards');
const observing = { activityRecord: { activityId: 'server', ownerTool: 'run_shell', surface: 'process', state: 'running' }, activity: event(6, 'stdout_chunk', { data: 'Building 2/3\n' }) };
assert(extractManagedProcess('activity_observe', observing)?.output === 'Building 2/3\n', 'active completion waits show process output before their final receipt');
assert(extractManagedProcess('activity_observe', { ...observing, activityRecord: { ...observing.activityRecord, ownerTool: 'browser_session', surface: 'browser' } }) === null, 'live browser observation envelopes remain outside the process card');

let bounded: ActivityEvent[] | undefined;
for (let seq = 0; seq < 200; seq++) bounded = mergeToolActivityEvents(bounded, { activity: event(seq, 'stdout_chunk', { data: 'x'.repeat(20_000) }) });
assert(bounded?.length === 128, 'retained activity count is bounded');
assert(bounded[0].payload.data === 'x'.repeat(8192), 'each output chunk is bounded');
assert(extractManagedProcess('run_shell', undefined, bounded)?.output.length === 8192, 'visible output tail is bounded');
assert(mergeToolActivityEvents(bounded, { activity: event(199, 'stdout_chunk', { data: 'last' }) })?.length === 128, 'replay events are deduplicated');
assert(extractManagedProcess('search', ready) === null, 'unrelated tools do not adopt process artifacts');
console.log('ok - process progress, readiness, failure, replay and bounded output contracts');
