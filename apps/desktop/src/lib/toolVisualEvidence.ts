import type { ArtifactPayload, ConversationMessage } from '../types/conversation';
import type { ToolCallEvent } from './streaming/protocol';
import { isNormalUserTurnMessage } from './streaming/chatVisibility';

export interface ToolVisualEvidence {
  name: string;
  mimeType: 'image/png' | 'image/jpeg' | 'image/webp';
  base64: string;
  contentHash?: string;
}

function record(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === 'object' && !Array.isArray(value));
}

function visualPayload(artifacts: ArtifactPayload | null | undefined): Record<string, unknown> | null {
  if (!record(artifacts)) return null;
  const value = artifacts.kind === 'toolVisualEvidence' ? artifacts : artifacts.visualEvidence;
  return record(value) && value.kind === 'toolVisualEvidence' && value.persistence === 'currentTurnOnly'
    ? value : null;
}

const MAX_RETAINED_IMAGES_PER_CONVERSATION = 12;
const MAX_RETAINED_IMAGE_BASE64_BYTES = 32 * 1024 * 1024;

/** Active conversations cannot be evicted by the outer message cache. Keep a
 * separate pixel budget, preserving message identity and durable receipts. */
function boundMessageVisualEvidence(messages: ConversationMessage[]): ConversationMessage[] {
  const budgets = new Map<string, { count: number; bytes: number }>();
  let bounded = messages;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    const payload = visualPayload(message.artifacts);
    const evidence = payload?.evidence;
    if (!record(evidence) || typeof evidence.base64 !== 'string' || !record(message.artifacts)) continue;
    const budget = budgets.get(message.conversationId) ?? { count: 0, bytes: 0 };
    budgets.set(message.conversationId, budget);
    const bytes = evidence.base64.length;
    if (budget.count < MAX_RETAINED_IMAGES_PER_CONVERSATION && budget.bytes + bytes <= MAX_RETAINED_IMAGE_BASE64_BYTES) {
      budget.count += 1;
      budget.bytes += bytes;
      continue;
    }
    if (bounded === messages) bounded = messages.slice();
    const artifacts = { ...message.artifacts };
    // Current messages use a nested visualEvidence field. Also support a raw
    // ephemeral envelope without removing its remaining receipt metadata.
    if (message.artifacts === payload) delete artifacts.evidence;
    else delete artifacts.visualEvidence;
    bounded[index] = { ...message, artifacts };
  }
  return bounded;
}

export function extractToolVisualEvidence(artifacts: ArtifactPayload | undefined): ToolVisualEvidence | null {
  const payload = visualPayload(artifacts);
  const evidence = payload?.evidence;
  if (!record(evidence)) return null;
  const { mimeType, base64 } = evidence;
  if (mimeType !== 'image/png' && mimeType !== 'image/jpeg' && mimeType !== 'image/webp') return null;
  if (typeof base64 !== 'string' || !base64.length || base64.length > 6 * 1024 * 1024
    || base64.length % 4 !== 0 || !/^[A-Za-z0-9+/]+={0,2}$/.test(base64)) return null;
  return {
    name: typeof evidence.name === 'string' && evidence.name.trim() ? evidence.name.trim() : 'visual-evidence',
    mimeType, base64,
    contentHash: typeof evidence.contentHash === 'string' ? evidence.contentHash : undefined,
  };
}

/** Merge a live screenshot independently of the durable result it illustrates.
 * This helper is for in-memory projections only; captures never enter storage. */
export function mergeToolArtifacts(
  previous: ArtifactPayload | undefined,
  incoming: ArtifactPayload | undefined,
): ArtifactPayload | undefined {
  const incomingVisual = visualPayload(incoming);
  const previousVisual = visualPayload(previous);
  if (incomingVisual && incomingVisual === incoming) {
    return { ...(record(previous) && previous.kind !== 'toolVisualEvidence' ? previous : {}), visualEvidence: incomingVisual };
  }
  if (!incoming) return previous;
  if (record(incoming) && previousVisual && !incomingVisual) {
    return { ...incoming, visualEvidence: previousVisual };
  }
  return incoming;
}

/** Carry live-only images across the UI handover to saved messages, not a reload.
 * Provider call IDs can repeat. Bound them to the owning user turn, then select
 * the latest matching result in that turn instead of rewriting historical rows. */
export function mergeCurrentTurnVisualEvidence(
  messages: ConversationMessage[],
  calls: Pick<ToolCallEvent, 'callId' | 'artifacts'>[],
  conversationId: string,
  userMessageId?: string | null,
): ConversationMessage[] {
  const evidenceByCall = new Map(calls.flatMap(call => {
    const evidence = visualPayload(call.artifacts);
    return evidence ? [[call.callId, evidence] as const] : [];
  }));
  if (!evidenceByCall.size) return boundMessageVisualEvidence(messages);
  let turnStart = -1;
  if (userMessageId) {
    turnStart = messages.findIndex(message => message.conversationId === conversationId && message.id === userMessageId && message.role === 'user');
  } else {
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      if (messages[index].conversationId === conversationId && isNormalUserTurnMessage(messages[index])) { turnStart = index; break; }
    }
  }
  // A missing owner is not permission to attach a capture to an older turn.
  if (turnStart < 0) return boundMessageVisualEvidence(messages);
  const followingTurn = messages.findIndex((message, index) => index > turnStart && message.conversationId === conversationId && isNormalUserTurnMessage(message));
  const turnEnd = followingTurn < 0 ? messages.length : followingTurn;
  const targets = new Map<string, number>();
  for (let index = turnEnd - 1; index > turnStart; index -= 1) {
    const message = messages[index];
    if (message.conversationId === conversationId && message.role === 'tool' && message.toolCallId && !targets.has(message.toolCallId)) targets.set(message.toolCallId, index);
  }
  let changed = false;
  const next = messages.map((message, index) => {
    if (!message.toolCallId || targets.get(message.toolCallId) !== index) return message;
    const evidence = evidenceByCall.get(message.toolCallId);
    if (!evidence || visualPayload(message.artifacts) === evidence) return message;
    changed = true;
    return { ...message, artifacts: mergeToolArtifacts(message.artifacts ?? undefined, evidence)! };
  });
  return boundMessageVisualEvidence(changed ? next : messages);
}

/** A refresh may contain several turns using the same provider call ID. Once
 * pixels belong to a saved row, retain them only for that exact message. */
export function retainMessageVisualEvidence(previous: ConversationMessage[], next: ConversationMessage[]): ConversationMessage[] {
  const byId = new Map(previous.filter(message => message.role === 'tool' && visualPayload(message.artifacts)).map(message => [message.id, message]));
  if (!byId.size) return boundMessageVisualEvidence(next);
  let changed = false;
  const merged = next.map(message => {
    const prior = byId.get(message.id);
    if (message.role !== 'tool' || !prior || prior.conversationId !== message.conversationId || prior.toolCallId !== message.toolCallId || visualPayload(message.artifacts)) return message;
    changed = true;
    return { ...message, artifacts: mergeToolArtifacts(message.artifacts ?? undefined, visualPayload(prior.artifacts)!)! };
  });
  return boundMessageVisualEvidence(changed ? merged : next);
}
