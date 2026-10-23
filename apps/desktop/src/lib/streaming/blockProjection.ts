import type { StreamTerminalProjectionState } from './terminalProjection';
import { blockTraceDelta, blockTraceText, replaceBlockTraceText, utf8ByteLength, type TextTraceEvent } from './blockTrace';

function setTextTrace(state: StreamTerminalProjectionState, index: number, event: TextTraceEvent): void {
  if (state._tools) {
    state._tools.setTextTrace(index, event);
  } else {
    const next = state.traceEvents.slice();
    next[index] = event;
    state.traceEvents = next;
  }
}

export function appendThinkingTraceEvent(state: StreamTerminalProjectionState, delta: string): void {
  if (!delta) return;
  const last = state.traceEvents[state.traceEvents.length - 1];
  if (last?.kind === 'thinking') {
    setTextTrace(state, state.traceEvents.length - 1, { ...last, text: last.text + delta });
    return;
  }

  setTextTrace(state, state.traceEvents.length, {
    id: `trace-thinking-${Date.now()}-${state._traceSeq++}`,
    kind: 'thinking',
    text: delta,
  });
}

export function appendReplyTraceEvent(state: StreamTerminalProjectionState, delta: string): void {
  if (!delta) return;
  const last = state.traceEvents[state.traceEvents.length - 1];
  if (last?.kind === 'reply') {
    setTextTrace(state, state.traceEvents.length - 1, { ...last, text: last.text + delta });
    return;
  }

  setTextTrace(state, state.traceEvents.length, {
    id: `trace-reply-${Date.now()}-${state._traceSeq++}`,
    kind: 'reply',
    text: delta,
  });
}

function pendingBlocks(
  state: StreamTerminalProjectionState,
  channel: 'answer' | 'thinking',
): Map<string, Map<number, string>> {
  return channel === 'answer'
    ? state._pendingAnswerBlockDeltas
    : state._pendingThinkingBlockDeltas;
}

function bufferBlockDelta(
  state: StreamTerminalProjectionState,
  channel: 'answer' | 'thinking',
  blockId: string,
  offset: number,
  delta: string,
): void {
  const blocks = pendingBlocks(state, channel);
  const pending = blocks.get(blockId) ?? new Map<number, string>();
  if (!pending.has(offset)) pending.set(offset, delta);
  blocks.set(blockId, pending);
}

function takeBufferedBlockDelta(
  state: StreamTerminalProjectionState,
  channel: 'answer' | 'thinking',
  blockId: string,
  offset: number,
): string | null {
  const blocks = pendingBlocks(state, channel);
  const pending = blocks.get(blockId);
  const delta = pending?.get(offset);
  if (delta === undefined) return null;
  pending?.delete(offset);
  if (pending?.size === 0) blocks.delete(blockId);
  return delta;
}

function upsertBlockTraceEvent(
  state: StreamTerminalProjectionState,
  kind: 'reply' | 'thinking',
  blockId: string,
  offset: number,
  nextOffset: number,
  delta: string,
): void {
  const update = blockTraceDelta(state.traceEvents, kind, blockId, offset, nextOffset, delta,
    () => `trace-${kind}-${Date.now()}-${state._traceSeq++}`);
  if (update) setTextTrace(state, update.index, update.event);
}

export function applyStreamBlockDelta(
  state: StreamTerminalProjectionState,
  channel: 'answer' | 'thinking',
  blockId: string,
  offset: number,
  delta: string,
): void {
  if (!blockId || !delta) return;
  const normalizedOffset = Number.isFinite(offset) && offset >= 0 ? offset : 0;

  if (channel === 'answer') {
    state.isThinking = false;
    if (state._activeRoundId) {
      state._activeRoundId = null;
      state._activeRoundAcceptingStarts = false;
    }
    if (state._activeAnswerBlockId !== blockId) {
      if (normalizedOffset !== 0) {
        bufferBlockDelta(state, channel, blockId, normalizedOffset, delta);
        return;
      }
      state._activeAnswerBlockId = blockId;
      state._activeAnswerOffset = 0;
      state.streamText = '';
    }
    if (normalizedOffset < state._activeAnswerOffset) return;
    if (normalizedOffset > state._activeAnswerOffset) {
      bufferBlockDelta(state, channel, blockId, normalizedOffset, delta);
      return;
    }
    // A tool can move the visible prefix into a round without ending the
    // provider's block. Resume that prefix instead of buffering forever or
    // displaying only the suffix when thought/tool events interleave.
    if (!state.streamText && state._activeAnswerOffset > 0) {
      state.streamText = blockTraceText(state.traceEvents, 'reply', blockId);
    }
    state.thinkingText = '';
    let nextDelta: string | null = delta;
    while (nextDelta !== null) {
      const nextOffset = state._activeAnswerOffset;
      state.streamText += nextDelta;
      state._activeAnswerOffset = nextOffset + utf8ByteLength(nextDelta);
      upsertBlockTraceEvent(state, 'reply', blockId, nextOffset, state._activeAnswerOffset, nextDelta);
      nextDelta = takeBufferedBlockDelta(
        state,
        channel,
        blockId,
        state._activeAnswerOffset,
      );
    }
    return;
  }

  state.isThinking = true;
  if (state._activeThinkingBlockId !== blockId) {
    if (normalizedOffset !== 0) {
      bufferBlockDelta(state, channel, blockId, normalizedOffset, delta);
      return;
    }
    state._activeThinkingBlockId = blockId;
    state._activeThinkingOffset = 0;
    state.thinkingText = '';
  }
  if (normalizedOffset < state._activeThinkingOffset) return;
  if (normalizedOffset > state._activeThinkingOffset) {
    bufferBlockDelta(state, channel, blockId, normalizedOffset, delta);
    return;
  }
  if (!state.thinkingText && state._activeThinkingOffset > 0) {
    state.thinkingText = blockTraceText(state.traceEvents, 'thinking', blockId);
  }
  let nextDelta: string | null = delta;
  while (nextDelta !== null) {
    const nextOffset = state._activeThinkingOffset;
    state.thinkingText += nextDelta;
    state._activeThinkingOffset = nextOffset + utf8ByteLength(nextDelta);
    upsertBlockTraceEvent(state, 'thinking', blockId, nextOffset, state._activeThinkingOffset, nextDelta);
    nextDelta = takeBufferedBlockDelta(
      state,
      channel,
      blockId,
      state._activeThinkingOffset,
    );
  }
}

/** A full upstream record supersedes only its block, including any stale gaps. */
export function applyStreamBlockSnapshot(
  state: StreamTerminalProjectionState,
  channel: 'answer' | 'thinking',
  blockId: string,
  text: string,
): void {
  if (!blockId) return;
  const kind = channel === 'answer' ? 'reply' : 'thinking';
  const index = state.traceEvents.findIndex(event => event.kind === kind && event.blockId === blockId);
  const nextOffset = utf8ByteLength(text);
  pendingBlocks(state, channel).delete(blockId);
  if (index >= 0) {
    state.traceEvents = replaceBlockTraceText(state.traceEvents, kind, blockId, text);
  } else {
    state.traceEvents = [...state.traceEvents, {
      id: `trace-${kind}-${Date.now()}-${state._traceSeq++}`,
      kind, blockId, text, startOffset: 0, nextOffset,
    }];
  }
  // Historical corrections must not steal the currently streaming block.
  if (channel === 'answer' && (index < 0 || state._activeAnswerBlockId === blockId)) {
    state._activeAnswerBlockId = blockId;
    state._activeAnswerOffset = nextOffset;
    state.streamText = text;
    state.isThinking = false;
    state.thinkingText = '';
    state._activeRoundId = null;
    state._activeRoundAcceptingStarts = false;
  } else if (channel === 'thinking' && (index < 0 || state._activeThinkingBlockId === blockId)) {
    state._activeThinkingBlockId = blockId;
    state._activeThinkingOffset = nextOffset;
    state.thinkingText = text;
    state.isThinking = true;
  }
}
