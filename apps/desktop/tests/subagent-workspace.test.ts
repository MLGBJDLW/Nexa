import { SubagentJournalProjection } from '../src/lib/subagentWorkspace';
import type { ActivityEvent } from '../src/types/conversation';
function assert(condition: unknown, message: string): asserts condition { if (!condition) throw new Error(message); }
let seq = 0;
const event = (subagentEvent: string, detail: unknown): ActivityEvent => ({ activityId: 'worker', seq: ++seq, timestamp: '', kind: 'progress', payload: { subagentEvent, detail } });
const stream = (value: unknown) => event('stream', { event: value });
const projection = new SubagentJournalProjection();
projection.append([
  event('spawned', { task: 'Review source' }),
  stream({ type: 'streamBlockSnapshot', channel: 'thinking', blockId: 'legacy-worker-thinking-1', text: 'legacy snapshot duplicate', legacy: true }),
  event('thinkingDelta', { delta: 'compatibility duplicate' }),
  stream({ type: 'streamBlockDelta', channel: 'thinking', blockId: 'think', offset: 0, delta: '你好' }),
  event('thinkingDelta', { delta: '你好' }),
  stream({ type: 'streamBlockDelta', channel: 'thinking', blockId: 'think', offset: 6, delta: '，继续' }),
  stream({ type: 'streamBlockSnapshot', channel: 'answer', blockId: 'answer', text: 'First answer' }),
]);
assert(projection.state.traceEvents.filter(item => item.kind === 'thinking').length === 1, 'canonical reasoning deduplicates compatibility events');
assert(projection.state.traceEvents.some(item => item.kind === 'thinking' && item.text === '你好，继续'), 'UTF-8 offsets preserve reasoning');
const completedTool = { callId: 'tool', toolName: 'read_file', status: 'completed', content: 'source text', arguments: '{"path":"a.rs"}' };
projection.append([event('progress', { run: completedTool }), stream({ type: 'streamBlockSnapshot', channel: 'answer', blockId: 'reject', text: 'Rejected draft' }), stream({ type: 'streamReset', reason: 'retry', discard_sample: true })]);
assert(!projection.state.traceEvents.some(item => item.kind === 'reply' && item.text === 'Rejected draft'), 'discarded samples vanish');
assert(projection.state.traceEvents.some(item => item.kind === 'reply' && item.text === 'First answer'), 'committed earlier round survives reset');
assert(projection.state.toolCalls[0]?.content === 'source text', 'tool output is retained');
const page = [event('inputQueued', { content: 'Check tests' }), event('inputApplied', { content: 'Check tests' }), event('completed', { result: { result: 'All checked' } })];
projection.append(page); const length = projection.state.traceEvents.length;
projection.append(page);
assert(projection.state.traceEvents.length === length, 'duplicate pages do not duplicate transcript');
assert(projection.result === 'All checked', 'terminal result is available after parent completion');
const legacy = new SubagentJournalProjection();
legacy.append([stream({ type: 'streamBlockSnapshot', channel: 'answer', blockId: 'legacy-worker-answer-1', text: 'private', legacy: true }),
  stream({ type: 'streamBlockSnapshot', channel: 'answer', blockId: 'legacy-worker-answer-1', text: '[PRIVATE]', legacy: true })]);
assert(legacy.state.traceEvents.length === 1 && legacy.state.traceEvents[0].kind === 'reply' && legacy.state.traceEvents[0].text === '[PRIVATE]', 'full legacy snapshots replace fragments after privacy redaction');

for (const kind of ['completed', 'failed', 'cancelled'] as const) {
  const child = new SubagentJournalProjection();
  child.append([event('progress', { run: { callId: 'unfinished', toolName: 'read_file', status: 'running', arguments: '{}' } })]);
  child.append([{ activityId: 'worker', seq: ++seq, timestamp: '', kind: kind === 'completed' ? 'completed' : kind === 'failed' ? 'failed' : 'cancelled', payload: {
    state: kind, detail: { agentId: 'worker', subagentEvent: kind, detail: { result: { result: 'Wrapped terminal result' }, errorMessage: kind === 'failed' ? 'Child provider failed' : null } },
  } }]);
  assert(child.result === 'Wrapped terminal result', `${kind} transition envelope preserves the final result`);
  assert(child.state.toolCalls[0]?.status === (kind === 'completed' ? 'done' : kind === 'failed' ? 'error' : 'cancelled'), `${kind} settles an unfinished child tool`);
  if (kind === 'failed') assert(child.state.traceEvents.some(item => item.kind === 'status' && item.text === 'Child provider failed'), 'failed transition exposes error details');
}

console.log('Subagent workspace projection contracts passed');
