import { createDefaultState } from '../src/lib/streaming/state';
import { applyStreamBlockDelta } from '../src/lib/streaming/blockProjection';
import { applyDoneEvent, applyStatusEvent } from '../src/lib/streaming/liveProjection';
import { applyStreamResetProjection } from '../src/lib/streaming/terminalProjection';
import { projectLiveConversationTimeline } from '../src/lib/streaming/timelineViewModel';
import { projectChatStreamingVisibility } from '../src/lib/streaming/chatVisibility';
import { createToolCall, insertPendingToolCall } from '../src/lib/streaming/toolProjection';

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

// A provider may emit the final answer only in Done after a steering restart.
// It must not replace an earlier, already completed reply in the same run.
for (const finalDelta of [false, true]) {
  const state = createDefaultState();
  state.isStreaming = true;
  applyStreamBlockDelta(state, 'answer', 'before-steering', 0, 'Earlier progress');
  applyStatusEvent(state, 'I was only asking', 'muted', 'user', 'steering');
  applyStreamResetProjection(state, 'steering_restart');
  applyStreamBlockDelta(state, 'thinking', 'after-steering', 0, 'Review the correction');
  if (finalDelta) applyStreamBlockDelta(state, 'answer', 'final-answer', 0, 'Final');
  applyDoneEvent(state, { status: 'completed', message: { content: 'Final summary' } });

  const replies = state.traceEvents.filter(event => event.kind === 'reply');
  assert(replies.length === 2, `finalDelta=${finalDelta}: earlier progress must survive Done`);
  assert(replies[0].text === 'Earlier progress', 'Done must not overwrite earlier progress');
  assert(replies[1].text === 'Final summary', 'Done remains the final-answer authority');
  const projection = projectLiveConversationTimeline({ ...state, displayedText: state.streamText });
  const final = projection.liveTraceTimeline[projection.liveTraceTimeline.length - 1];
  assert(final?.kind === 'reply' && final.content === 'Final summary', 'final reply must follow steering and thinking');
  const visible = projectChatStreamingVisibility(state);
  assert(visible.streamRounds.length === 0, 'Done must keep the ordered trace authoritative over rounds');
}

// Preparing a tool retires the streamed text before it clears the block ID.
// Done must still append after that tool rather than rewrite the old block.
{
  const state = createDefaultState();
  state.isStreaming = true;
  applyStreamBlockDelta(state, 'answer', 'before-tool', 0, 'Earlier progress');
  insertPendingToolCall(state, createToolCall({
    callId: 'tool-1', toolName: 'read_file', arguments: '{}', status: 'preparing',
  }), '');
  applyDoneEvent(state, { status: 'completed', message: { content: 'Final summary' } });
  const replies = state.traceEvents.filter(event => event.kind === 'reply');
  assert(replies.length === 2 && replies[0].text === 'Earlier progress', 'tool boundary retains earlier reply');
  assert(state.traceEvents[state.traceEvents.length - 1] === replies[1], 'Done appends after the tool boundary');
}

console.log('Terminal reply ordering contracts passed');
