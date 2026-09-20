import type { ConversationMessage } from '../../types/conversation';
import type { StreamRoundEvent, TraceEvent } from './protocol';
import {
  isGoalMessage,
  isOptimisticSteeringMessage,
  isSteeringMessage,
} from '../chatMessageGuards';

export interface ChatStreamingVisibilityInput {
  isStreaming: boolean;
  streamRounds: StreamRoundEvent[];
  traceEvents: TraceEvent[];
}

export interface ChatStreamingVisibilityProjection {
  streamRounds: StreamRoundEvent[];
  traceEvents: TraceEvent[];
  strategy: 'default' | 'traceTimeline';
}

export interface ChatMessageVisibilityInput {
  isStreaming: boolean;
  messages: ConversationMessage[];
}

export interface ChatMessageVisibilityProjection {
  historyMessages: ConversationMessage[];
  liveSteeringMessages: ConversationMessage[];
}

function artifactContainsKind(value: unknown, kind: string, depth = 0): boolean {
  if (depth > 6 || value == null) return false;
  if (Array.isArray(value)) {
    return value.some((item) => artifactContainsKind(item, kind, depth + 1));
  }
  if (typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  if (record.kind === kind) return true;
  return Object.values(record).some((item) => artifactContainsKind(item, kind, depth + 1));
}

export function isQuestionResponseMessage(message: ConversationMessage): boolean {
  return message.role === 'user' && artifactContainsKind(message.artifacts, 'questionResponse');
}

export function isCheckpointContinuationMessage(message: ConversationMessage): boolean {
  return message.role === 'user'
    && artifactContainsKind(message.artifacts, 'checkpointContinuation');
}

function isNormalUserTurnMessage(message: ConversationMessage): boolean {
  return (
    message.role === 'user' &&
    !isSteeringMessage(message) &&
    !isGoalMessage(message) &&
    !isQuestionResponseMessage(message) &&
    !isCheckpointContinuationMessage(message)
  );
}

export function hasPersistedResultAfterLatestUserMessage(
  messages: ConversationMessage[],
): boolean {
  let lastUserIdx = -1;
  for (let idx = messages.length - 1; idx >= 0; idx -= 1) {
    if (isNormalUserTurnMessage(messages[idx])) {
      lastUserIdx = idx;
      break;
    }
  }
  if (lastUserIdx < 0) return false;
  return messages
    .slice(lastUserIdx + 1)
    .some(message => message.role === 'assistant' || message.role === 'tool');
}

/**
 * ChatMessages renders live trace and completed stream rounds through two
 * separate paths. The live trace path intentionally trims events that have
 * already been materialized as `streamRounds`; however, the component also
 * hides `streamRounds` whenever a live trace timeline exists. During a new
 * streaming thinking phase this made earlier in-turn replies/thinking vanish
 * until the final persisted replay replaced the live state.
 *
 * Keep the ordered trace authoritative through Done and the durable-history
 * handoff. Switching back to rounds at Done appends any remaining steering and
 * thinking after the final reply, since rounds do not carry user boundaries.
 * The same projection therefore owns both active and completed previews.
 */
export function projectChatStreamingVisibility(
  input: ChatStreamingVisibilityInput,
): ChatStreamingVisibilityProjection {
  if (
    input.streamRounds.length > 0 &&
    input.traceEvents.length > 0
  ) {
    return {
      streamRounds: [],
      traceEvents: input.traceEvents,
      strategy: 'traceTimeline',
    };
  }

  return {
    streamRounds: input.streamRounds,
    traceEvents: input.traceEvents,
    strategy: 'default',
  };
}

/**
 * Steering and structured continuations are control-plane events, not ordinary
 * chat turns. Live steering is placed by the ordered event timeline; completed
 * steering remains at its durable message position. Question responses and
 * checkpoint continuation prompts remain in the projected collection as
 * system rows so durable replay can consume them while normal bubble rendering
 * omits them.
 */
export function projectChatMessageVisibility(
  input: ChatMessageVisibilityInput,
): ChatMessageVisibilityProjection {
  let latestUserIndex = -1;
  for (let index = input.messages.length - 1; index >= 0; index--) {
    if (isNormalUserTurnMessage(input.messages[index])) { latestUserIndex = index; break; }
  }
  return {
    historyMessages: input.messages
      .filter((message, index) => !isSteeringMessage(message)
        || ((!input.isStreaming || index < latestUserIndex) && !isOptimisticSteeringMessage(message)))
      .map(message => (
        isQuestionResponseMessage(message) || isCheckpointContinuationMessage(message)
      )
        ? { ...message, role: 'system' as const }
        : message),
    liveSteeringMessages: input.isStreaming
      ? input.messages.filter(isOptimisticSteeringMessage)
      : [],
  };
}
