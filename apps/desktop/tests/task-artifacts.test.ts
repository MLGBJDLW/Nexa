import { extractSubtaskArtifacts, findLatestSubtaskArtifacts } from '../src/lib/taskArtifacts';
import type { ToolCallEvent } from '../src/lib/streaming/protocol';

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const ordinaryRuntimeArtifacts = {
  kind: 'agentTaskArtifacts',
  subtasks: [],
  selectedSkills: {
    kind: 'selectedSkills',
    skills: [
      { id: 'skill-docs', name: 'Docs', enabled: true },
    ],
  },
  workflow: {
    runs: [
      { id: 'workflow-1', label: 'Build report', status: 'done' },
    ],
  },
  tools: [
    { id: 'tool-1', label: 'Search files', status: 'done' },
  ],
};

assert(
  extractSubtaskArtifacts(ordinaryRuntimeArtifacts).length === 0,
  'skills, workflows, and ordinary tools must not be projected as subagents',
);

const realSubtasks = extractSubtaskArtifacts({
  kind: 'agentTaskArtifacts',
  subtasks: [
    {
      id: 'subtask-1',
      parentRunId: 'run-1',
      label: 'Research implementation',
      role: 'researcher',
      status: 'completed',
    },
  ],
});
assert(realSubtasks.length === 1, 'canonical subtask rows should remain visible');
assert(realSubtasks[0].id === 'subtask-1', 'real subtask identity should be preserved');

console.log('ok - task artifacts require explicit subagent provenance');

const longTask = 'Audit the chat renderer.\n' + 'Detailed private task instructions. '.repeat(100);
const spawn: ToolCallEvent = {
  callId: 'spawn-call', toolName: 'spawn_subagent', arguments: '{}', status: 'done',
  argsStatus: 'done', argsBytes: 2, content: '', isError: false,
  artifacts: { kind: 'subagent_result', id: 'agent-one', status: 'running', task: longTask, result: '', toolEvents: [] },
};
const closed: ToolCallEvent = {
  ...spawn, callId: 'close-call', toolName: 'close_subagent',
  artifacts: { kind: 'subagent_closed', worker: {
    agentId: 'agent-one', parentCallId: 'spawn-call', task: longTask, status: 'completed', result: null,
  } },
};
const closedRuns = findLatestSubtaskArtifacts([], [spawn, closed]);
assert(closedRuns.length === 1 && closedRuns[0].status === 'completed', 'closed worker snapshots must replace stale spawn status in the capsule');
assert(closedRuns[0].label.length <= 80 && !closedRuns[0].label.includes('Detailed private'), 'capsule title must be a short first-line task label, not the full prompt');

const queuedInput: ToolCallEvent = { ...spawn, callId: 'steer-call', toolName: 'send_subagent_input',
  artifacts: { kind: 'subagent_input_queued', agentId: 'agent-one', state: 'queued' } };
const steered = findLatestSubtaskArtifacts([], [spawn, queuedInput]);
assert(steered.length === 1 && steered[0].status === 'running', 'an input enqueue receipt is not worker completion');
const observed: ToolCallEvent = { ...spawn, callId: 'observe-call', toolName: 'observe_subagent', artifacts: {
  kind: 'subagent_observation', observation: { worker: { agentId: 'agent-one', task: longTask, status: 'cancelled' }, cursor: 7, events: [], timedOut: false },
} };
const observedRuns = findLatestSubtaskArtifacts([], [spawn, queuedInput, observed]);
assert(observedRuns.length === 1 && observedRuns[0].status === 'cancelled', 'wrapped observation snapshots must update the existing worker');

const durable = { subtasks: [{ id: 'durable-row', parentRunId: 'parent', label: 'agent-one',
  input: { kind: 'subagent_input', callLabel: 'agent-one', task: longTask }, status: 'completed', role: 'researcher' }] };
const reconciled = findLatestSubtaskArtifacts([], [spawn], durable);
assert(reconciled.length === 1 && reconciled[0].status === 'completed', 'durable and lifecycle identities must merge without resurrecting a completed worker');

const live: ToolCallEvent = { ...spawn, activityEvents: [
  { activityId: 'agent-one', seq: 1, timestamp: '', kind: 'state_changed', payload: { subagentEvent: 'spawned', agentId: 'agent-one', detail: { task: longTask } } },
  { activityId: 'agent-one', seq: 2, timestamp: '', kind: 'cancelled', payload: { subagentEvent: 'cancelled', agentId: 'agent-one', detail: { status: 'cancelled' } } },
] };
const cancelled = findLatestSubtaskArtifacts([], [live]);
assert(cancelled.length === 1 && cancelled[0].status === 'cancelled', 'post-spawn activity events must settle the capsule without waiting for the parent answer');
const replayed = findLatestSubtaskArtifacts([], [closed, spawn, { ...live, activityEvents: live.activityEvents?.slice(0, 1) }]);
assert(replayed.length === 1 && replayed[0].status === 'completed', 'late running snapshots or spawned events cannot resurrect a closed worker');

const batch: ToolCallEvent = { ...spawn, callId: 'batch', toolName: 'spawn_subagent_batch', artifacts: {
  kind: 'subagent_batch_result', lifecycleWorkers: [{ agentId: 'agent-batch', workerId: 'reviewer', task: 'Review' }],
  runs: [{ id: 'reviewer', task: 'Review', status: 'done', result: 'Checked' }],
} };
const batchClosed: ToolCallEvent = { ...closed, artifacts: { kind: 'subagent_closed', worker: {
  agentId: 'agent-batch', task: 'Review', status: 'completed', result: { id: 'reviewer', result: 'Checked' },
} } };
const batchRuns = findLatestSubtaskArtifacts([], [batch, batchClosed]);
assert(batchRuns.length === 1 && batchRuns[0].status === 'completed', 'batch labels and lifecycle IDs must describe one worker');
console.log('ok - task capsule uses terminal worker state and compact task labels');
