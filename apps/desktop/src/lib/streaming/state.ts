import type { AgentRunEvent } from '../../types/conversation';
import type { StreamState } from './protocol';
import type { StreamTimeoutHandle } from './watchdog';
import { isPendingToolCallStatus } from './toolStatus';
import { StreamToolEntities } from './toolEntities';

export interface InternalStreamState extends StreamState {
  _tools: StreamToolEntities;
  _terminalRunId: string | null;
  _toolCallSeq: number;
  _roundSeq: number;
  _traceSeq: number;
  _orderedRunId: string | null;
  _lastEventSeq: number;
  _pendingRunEvents: Map<number, AgentRunEvent>;
  _activeAnswerBlockId: string | null;
  _activeAnswerOffset: number;
  _activeThinkingBlockId: string | null;
  _activeThinkingOffset: number;
  _pendingAnswerBlockDeltas: Map<string, Map<number, string>>;
  _pendingThinkingBlockDeltas: Map<string, Map<number, string>>;
  _activeRoundId: string | null;
  _activeRoundAcceptingStarts: boolean;
  _timeoutId: StreamTimeoutHandle | null;
  _watchdogGeneration: number;
  _watchdogRecoveryAttempt: number;
  _watchdogMissingRunConfirmations: number;
  _toolPreparingTimers: Record<string, ReturnType<typeof setTimeout>>;
  _launchStartedAt: number | null;
  _frontendPaintScheduled: boolean;
  _frontendPaintReported: boolean;
}

export function createDefaultState(): InternalStreamState {
  const tools = new StreamToolEntities();
  return {
    _tools: tools,
    _terminalRunId: null,
    turnHandle: null,
    isStreaming: false,
    streamText: '',
    get streamRounds() { return tools.streamRounds; },
    set streamRounds(value) { tools.streamRounds = value; },
    get traceEvents() { return tools.traceEvents; },
    set traceEvents(value) { tools.traceEvents = value; },
    thinkingText: '',
    isThinking: false,
    get toolCalls() { return tools.toolCalls; },
    set toolCalls(value) { tools.toolCalls = value; },
    error: null,
    lastUsage: null,
    lastCached: false,
    finishReason: null,
    contextOverflow: false,
    rateLimited: false,
    connectionState: null,
    autoCompacted: null,
    pendingApprovals: [],
    taskRun: null,
    taskEvents: [],
    turnTiming: null,
    _toolCallSeq: 0,
    _roundSeq: 0,
    _traceSeq: 0,
    _orderedRunId: null,
    _lastEventSeq: 0,
    _pendingRunEvents: new Map(),
    _activeAnswerBlockId: null,
    _activeAnswerOffset: 0,
    _activeThinkingBlockId: null,
    _activeThinkingOffset: 0,
    _pendingAnswerBlockDeltas: new Map(),
    _pendingThinkingBlockDeltas: new Map(),
    _activeRoundId: null,
    _activeRoundAcceptingStarts: false,
    _timeoutId: null,
    _watchdogGeneration: 0,
    _watchdogRecoveryAttempt: 0,
    _watchdogMissingRunConfirmations: 0,
    _toolPreparingTimers: {},
    _launchStartedAt: null,
    _frontendPaintScheduled: false,
    _frontendPaintReported: false,
  };
}

export function clearToolPreparingTimer(state: InternalStreamState, callId: string): void {
  const timer = state._toolPreparingTimers[callId];
  if (!timer) return;
  clearTimeout(timer);
  delete state._toolPreparingTimers[callId];
}

export function clearToolPreparingTimers(state: InternalStreamState): void {
  Object.values(state._toolPreparingTimers).forEach(timer => clearTimeout(timer));
  state._toolPreparingTimers = {};
}

export function capStreamCollections(state: InternalStreamState): void {
  let trimmed = false;
  if (state._tools.traceCount > 512) { state.traceEvents = state.traceEvents.slice(-512); trimmed = true; }
  if (state._tools.roundCount > 128) { state.streamRounds = state.streamRounds.slice(-128); trimmed = true; }
  if (state.taskEvents.length > 256) state.taskEvents = state.taskEvents.slice(-256);
  if (state._tools.activeCount > 512) {
    const retained = new Set(state.traceEvents.flatMap(event => event.kind === 'tool' ? [event.toolCall.callId] : []));
    for (const round of state.streamRounds) for (const tool of round.toolCalls) retained.add(tool.callId);
    state.toolCalls = state.toolCalls.filter(tool => retained.has(tool.callId)
      || isPendingToolCallStatus(tool.status));
    trimmed = true;
  }
  if (trimmed) state._tools.prune();
}
