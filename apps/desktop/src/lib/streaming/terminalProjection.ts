import type {
  StreamRoundEvent,
  ToolCallEvent,
  TraceEvent,
  TraceStatusEvent,
  TraceToolEvent,
} from './protocol';
import {
  isPendingToolCallStatus,
  type TerminalToolStatus,
} from './toolStatus';
import type { StreamToolEntities } from './toolEntities';

export interface StreamTerminalProjectionState {
  _tools?: StreamToolEntities;
  pendingApprovals?: unknown[];
  isStreaming: boolean;
  streamText: string;
  streamRounds: StreamRoundEvent[];
  traceEvents: TraceEvent[];
  thinkingText: string;
  isThinking: boolean;
  toolCalls: ToolCallEvent[];
  error: string | null;
  _traceSeq: number;
  _activeAnswerBlockId: string | null;
  _activeAnswerOffset: number;
  _activeThinkingBlockId: string | null;
  _activeThinkingOffset: number;
  _pendingAnswerBlockDeltas: Map<string, Map<number, string>>;
  _pendingThinkingBlockDeltas: Map<string, Map<number, string>>;
  _activeRoundId: string | null;
  _activeRoundAcceptingStarts: boolean;
}

export function markToolCallsFinished(
  toolCalls: ToolCallEvent[],
  status: TerminalToolStatus,
  fallbackContent: string,
): ToolCallEvent[] {
  return toolCalls.map(tc =>
    isPendingToolCallStatus(tc.status)
      ? {
          ...tc,
          status,
          argsStatus: status === 'error' || status === 'timedOut' ? 'error' : 'done',
          content: tc.content || fallbackContent,
          isError: status === 'error' || status === 'timedOut',
        }
      : tc,
  );
}

export function markRoundsToolCallsFinished(
  rounds: StreamRoundEvent[],
  status: TerminalToolStatus,
  fallbackContent: string,
): StreamRoundEvent[] {
  return rounds.map(round => ({
    ...round,
    toolCalls: markToolCallsFinished(round.toolCalls, status, fallbackContent),
  }));
}

export function appendStatusTraceEvent(
  state: StreamTerminalProjectionState,
  text: string,
  tone: TraceStatusEvent['tone'] = 'muted',
  visibility: TraceStatusEvent['visibility'] = 'user',
  displayKind: TraceStatusEvent['displayKind'] = 'status',
  code?: string,
): void {
  if (!text.trim()) return;
  state.traceEvents = [...state.traceEvents, {
    id: `trace-status-${Date.now()}-${state._traceSeq++}`,
    kind: 'status',
    text,
    tone,
    visibility,
    displayKind,
    code,
  }];
}

export function clearTransientControllerStatus(
  state: StreamTerminalProjectionState,
  code: string,
): void {
  state.traceEvents = state.traceEvents.filter(event => (
    event.kind !== 'status' || event.code !== code
  ));
}

export function syncTraceToolEvents(state: StreamTerminalProjectionState): void {
  if (state._tools) return; // ID projections already resolve the current entity.
  state.traceEvents = state.traceEvents.map(event => {
    if (event.kind !== 'tool') return event;
    const latest = state.toolCalls.find(tc => tc.callId === event.toolCall.callId);
    return latest ? { ...event, toolCall: latest } as TraceToolEvent : event;
  });
}

export function finishProjectedTools(state: StreamTerminalProjectionState, status: TerminalToolStatus, fallbackContent: string): void {
  if (state._tools) {
    for (const [id,tool] of state._tools.byId) {
      if (!isPendingToolCallStatus(tool.status)) continue;
      state._tools.set({ ...tool, status, argsStatus: status === 'error' || status === 'timedOut' ? 'error' : 'done', content: tool.content || fallbackContent, isError: status === 'error' || status === 'timedOut' },id);
    }
    return;
  }
  state.toolCalls = markToolCallsFinished(state.toolCalls,status,fallbackContent);
  state.streamRounds = markRoundsToolCallsFinished(state.streamRounds,status,fallbackContent);
  syncTraceToolEvents(state);
}

export function resetActiveStreamBlocks(state: StreamTerminalProjectionState): void {
  state._activeAnswerBlockId = null;
  state._activeAnswerOffset = 0;
  state._activeThinkingBlockId = null;
  state._activeThinkingOffset = 0;
  state._pendingAnswerBlockDeltas.clear();
  state._pendingThinkingBlockDeltas.clear();
}

export function applyStreamResetProjection(
  state: StreamTerminalProjectionState,
  reason: string,
  options: { clearTools?: boolean; discardSample?: boolean } = {},
): void {
  state.streamText = '';
  state.thinkingText = '';
  state.isThinking = false;
  clearTransientControllerStatus(state, 'model_planning_slow');

  if (options.discardSample) {
    const pendingToolIds = new Set(
      state.toolCalls
        .filter(toolCall => isPendingToolCallStatus(toolCall.status))
        .map(toolCall => toolCall.callId),
    );
    const lastCommittedToolIndex = state.traceEvents.reduce(
      (latest, event, index) => event.kind === 'tool'
        && !pendingToolIds.has(event.toolCall.callId)
        && !isPendingToolCallStatus(event.toolCall.status)
        ? index
        : latest,
      -1,
    );
    state.traceEvents = state.traceEvents.filter((event, index) => {
      if (event.kind === 'tool' && pendingToolIds.has(event.toolCall.callId)) return false;
      return index <= lastCommittedToolIndex || (event.kind !== 'reply' && event.kind !== 'thinking');
    });
    state.toolCalls = state.toolCalls.filter(toolCall => !pendingToolIds.has(toolCall.callId));
    state.streamRounds = state.streamRounds.filter(round => (
      round.id !== state._activeRoundId
      && !round.toolCalls.some(toolCall => pendingToolIds.has(toolCall.callId))
    ));
  } else if (options.clearTools) {
    // A reset starts a new model attempt, but it should not erase the
    // already-rendered transcript for this turn. Keep prior trace/round UI
    // intact and make any interrupted tools terminal so the timeline does not
    // show stale in-progress work forever. The flat active-tool list is still
    // cleared so new tool calls after the reset start from a clean slate.
    finishProjectedTools(state, 'cancelled', reason || 'Interrupted by stream reset');
    state.toolCalls = [];
  }

  state.error = null;
  state._activeRoundId = null;
  state._activeRoundAcceptingStarts = false;
  state._tools?.prune();
  resetActiveStreamBlocks(state);
}

export function applyTerminalProjection(
  state: StreamTerminalProjectionState,
  input: {
    toolStatus: TerminalToolStatus;
    message: string;
    toolFallbackMessage?: string;
    traceTone: 'success' | 'error';
    errorMessage?: string | null;
  },
): void {
  state.isStreaming = false;
  state.pendingApprovals = [];
  state.isThinking = false;
  state.thinkingText = '';
  const toolFallbackMessage = input.toolFallbackMessage ?? input.message;
  finishProjectedTools(state, input.toolStatus, toolFallbackMessage);
  if (input.errorMessage !== undefined) state.error = input.errorMessage;
  appendStatusTraceEvent(state, input.message, input.traceTone);
  state._activeRoundId = null;
  state._activeRoundAcceptingStarts = false;
  resetActiveStreamBlocks(state);
}
