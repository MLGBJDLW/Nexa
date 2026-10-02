import type { ArtifactPayload, ToolRunItem } from '../../types/conversation';
import type { ToolCallEvent, TraceToolEvent } from './protocol';
import type { InternalStreamState } from './state';
import { mergeToolArtifacts } from '../toolVisualEvidence';
import { mergeToolActivityEvents } from '../processArtifacts';
import {
  resetActiveStreamBlocks,
  type StreamTerminalProjectionState,
} from './terminalProjection';
import {
  argsStatusForToolRun,
  isPendingToolCallStatus,
  toolRunStatusToToolCallStatus,
} from './toolStatus';

export type StreamToolProjectionState = InternalStreamState;

export interface ToolPreparingPayload {
  callId: string;
  toolName: string;
  argsBytes: number;
}

export function createToolCall(partial: {
  callId: string;
  toolName: string;
  arguments?: string;
  status?: ToolCallEvent['status'];
  argsStatus?: ToolCallEvent['argsStatus'];
  renderKind?: ToolCallEvent['renderKind'];
  capabilities?: ToolCallEvent['capabilities'];
  owner?: ToolCallEvent['owner'];
  providerExecuted?: boolean;
}): ToolCallEvent {
  const argumentsText = partial.arguments ?? '';
  return {
    callId: partial.callId,
    toolName: partial.toolName,
    owner: partial.owner,
    providerExecuted: partial.providerExecuted,
    arguments: argumentsText,
    status: partial.status ?? 'starting',
    renderKind: partial.renderKind,
    capabilities: partial.capabilities,
    argsStatus: partial.argsStatus ?? 'ready',
    argsBytes: argumentsText.length,
  };
}

function finalizeToolCall(
  toolCall: ToolCallEvent,
  isError: boolean | undefined,
  content: string | undefined,
  artifacts: ArtifactPayload | undefined,
): ToolCallEvent {
  return {
    ...toolCall,
    status: isError ? 'error' : 'done',
    argsStatus: isError ? 'error' : 'done',
    content,
    isError,
    artifacts: mergeToolArtifacts(toolCall.artifacts, artifacts),
    activityEvents: mergeToolActivityEvents(toolCall.activityEvents, artifacts),
  };
}

function patchToolCallFromRun(previous: ToolCallEvent, run: ToolRunItem): ToolCallEvent {
  const status = toolRunStatusToToolCallStatus(run.status);
  const argumentsText = run.arguments ?? previous.arguments;
  // The screenshot event is intentionally ephemeral. Merge it into the live
  // projection so it does not replace the durable result artifact that will
  // be restored when the conversation is reopened.
  const artifacts = mergeToolArtifacts(previous.artifacts, run.artifacts);
  return {
    ...previous,
    toolName: run.toolName || previous.toolName,
    arguments: argumentsText,
    status,
    renderKind: run.renderKind ?? previous.renderKind,
    capabilities: run.capabilities ?? previous.capabilities,
    owner: run.owner ?? previous.owner,
    providerExecuted: run.providerExecuted ?? previous.providerExecuted,
    argsStatus: argsStatusForToolRun(run, status),
    argsBytes: Math.max(previous.argsBytes, argumentsText.length),
    content: run.content ?? previous.content,
    isError: run.isError ?? previous.isError,
    artifacts,
    activityEvents: mergeToolActivityEvents(previous.activityEvents, run.artifacts),
    durationMs: run.durationMs ?? previous.durationMs,
    progressNote: run.progressNote ?? previous.progressNote,
  };
}

export function resolveToolCallResult(
  previous: ToolCallEvent[],
  resultCallId: string,
  resultIsError: boolean | undefined,
  resultContent: string | undefined,
  resultArtifacts: ArtifactPayload | undefined,
): { next: ToolCallEvent[]; matched: boolean } {
  let matched = false;
  const updated = previous.map(toolCall => {
    if (toolCall.callId !== resultCallId) return toolCall;
    matched = true;
    return finalizeToolCall(toolCall, resultIsError, resultContent, resultArtifacts);
  });
  if (matched) return { next: updated, matched: true };

  let fallbackIndex = -1;
  for (let index = updated.length - 1; index >= 0; index -= 1) {
    if (isPendingToolCallStatus(updated[index].status)) {
      fallbackIndex = index;
      break;
    }
  }
  if (fallbackIndex < 0) return { next: updated, matched: false };
  const copy = [...updated];
  copy[fallbackIndex] = finalizeToolCall(
    copy[fallbackIndex],
    resultIsError,
    resultContent,
    resultArtifacts,
  );
  return { next: copy, matched: true };
}

export function upsertToolTraceEvent(
  state: StreamTerminalProjectionState,
  toolCall: ToolCallEvent,
): void {
  const index = state.traceEvents.findIndex(event =>
    event.kind === 'tool' && (state._tools
      ? state._tools.key(event.toolCall) === state._tools.key(toolCall)
      : event.toolCall.callId === toolCall.callId));
  if (index >= 0) {
    const next = [...state.traceEvents];
    next[index] = { ...next[index], toolCall } as TraceToolEvent;
    state.traceEvents = next;
    return;
  }
  state.traceEvents = [...state.traceEvents, {
    id: `trace-tool-${Date.now()}-${state._traceSeq++}`,
    kind: 'tool',
    toolCall,
  }];
}

export function insertPendingToolCall(
  state: StreamToolProjectionState,
  toolCall: ToolCallEvent,
  roundThinking: string,
): void {
  if (state.streamText.trim().length > 0) {
    const roundId = `stream-round-${Date.now()}-${state._roundSeq++}`;
    state._activeRoundId = roundId;
    state._activeRoundAcceptingStarts = true;
    state.streamRounds = [...state.streamRounds, {
      id: roundId,
      thinking: roundThinking || undefined,
      reply: state.streamText,
      toolCalls: [toolCall],
    }];
    state.streamText = '';
  } else if (state._activeRoundId && state._activeRoundAcceptingStarts) {
    const roundId = state._activeRoundId;
    const target = state.streamRounds.find(round => round.id === roundId);
    if (target) {
      state.streamRounds = state.streamRounds.map(round => round.id === roundId
        ? {
            ...round,
            thinking: roundThinking ? `${round.thinking || ''}${roundThinking}` : round.thinking,
            toolCalls: [...round.toolCalls, toolCall],
          }
        : round);
    } else {
      state._activeRoundId = null;
      state._activeRoundAcceptingStarts = false;
      insertPendingToolCall(state, toolCall, roundThinking);
      return;
    }
  } else {
    const roundId = `stream-round-${Date.now()}-${state._roundSeq++}`;
    state._activeRoundId = roundId;
    state._activeRoundAcceptingStarts = true;
    state.streamRounds = [...state.streamRounds, {
      id: roundId,
      thinking: roundThinking || undefined,
      reply: '',
      toolCalls: [toolCall],
    }];
  }

  state.toolCalls = [...state.toolCalls, toolCall];
  upsertToolTraceEvent(state, toolCall);
}

export function applyToolRunEvent(state: StreamToolProjectionState, run: ToolRunItem): void {
  const callId = (run.callId || '').trim();
  if (!callId) return;
  const toolName = (run.toolName || '').trim() || 'unknown_tool';

  const existing = state._tools.get(callId);
  if (!existing || !state._tools.hasActive(callId)) {
    const roundThinking = state.thinkingText.trim() ? state.thinkingText : '';
    if (roundThinking) state.thinkingText = '';
    state.isThinking = false;

    const status = toolRunStatusToToolCallStatus(run.status);
    const base = createToolCall({
      callId,
      toolName,
      arguments: run.arguments ?? '',
      status,
      argsStatus: argsStatusForToolRun(run, status),
      renderKind: run.renderKind,
      capabilities: run.capabilities,
      owner: run.owner,
      providerExecuted: run.providerExecuted,
    });
    const nextCall = patchToolCallFromRun(base, run);
    insertPendingToolCall(state, nextCall, roundThinking);
    resetActiveStreamBlocks(state);
    if (!isPendingToolCallStatus(nextCall.status)) state._activeRoundAcceptingStarts = false;
    return;
  }

  const latest = patchToolCallFromRun(existing, run);
  state._tools.set(latest);
  if (!isPendingToolCallStatus(latest.status)) state._activeRoundAcceptingStarts = false;
}

export function toolPreparingPayloadFromRun(run: ToolRunItem): ToolPreparingPayload | null {
  const callId = (run.callId || '').trim();
  if (!callId || run.status !== 'preparing') return null;
  return {
    callId,
    toolName: (run.toolName || '').trim() || 'unknown_tool',
    argsBytes: typeof run.arguments === 'string' ? run.arguments.length : 0,
  };
}
