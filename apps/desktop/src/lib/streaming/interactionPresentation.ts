import type { ToolCallEvent } from './protocol';
import { isPendingToolCallStatus, isUnsuccessfulToolCallStatus } from './toolStatus';

const INTERACTION_TOOLS = new Set(['computer_observe', 'computer_control', 'desktop_automation', 'browser_session', 'browser_evidence_capture']);
export const isInteractionTool = (name: string): boolean => INTERACTION_TOOLS.has(name);

const presentations = {
  running: { labelKey: 'chat.toolRunning', tone: 'neutral', pending: true },
  pending: { labelKey: 'chat.interactionPending', tone: 'warning', pending: false },
  uncertain: { labelKey: 'chat.interactionUncertain', tone: 'warning', pending: false },
  notDispatched: { labelKey: 'chat.interactionNotDispatched', tone: 'warning', pending: false },
  unchanged: { labelKey: 'chat.interactionUnchanged', tone: 'neutral', pending: false },
  changed: { labelKey: 'chat.interactionChanged', tone: 'neutral', pending: false },
  targetClosed: { labelKey: 'chat.interactionTargetClosed', tone: 'neutral', pending: false },
  matched: { labelKey: 'chat.interactionMatched', tone: 'success', pending: false },
  observed: { labelKey: 'chat.interactionObserved', tone: 'neutral', pending: false },
  done: { labelKey: 'chat.toolBriefDone', tone: 'neutral', pending: false },
  error: { labelKey: 'chat.toolBriefError', tone: 'danger', pending: false },
  cancelled: { labelKey: 'chat.toolBriefCancelled', tone: 'danger', pending: false },
  declined: { labelKey: 'chat.toolBriefDeclined', tone: 'danger', pending: false },
} as const;
export type InteractionPresentation = typeof presentations[keyof typeof presentations];
type ToolInput = Pick<ToolCallEvent, 'toolName' | 'status'> & Partial<Pick<ToolCallEvent, 'arguments' | 'isError' | 'artifacts' | 'activityEvents' | 'capabilities'>>;
type RecordValue = Record<string, unknown>;
const record = (value: unknown): RecordValue | null => value && typeof value === 'object' && !Array.isArray(value) ? value as RecordValue : null;

/** Inspect only the tool's known output envelopes, never page text or arbitrary JSON. */
function envelopes(value: unknown, depth = 0): RecordValue[] {
  const root = record(value);
  if (!root || depth > 3) return [];
  return [root, ...['data', 'artifacts', 'toolOutput'].flatMap(key => envelopes(root[key], depth + 1))];
}

function canonicalReceipt(value: unknown): RecordValue | null {
  const receipt = record(value);
  return receipt && ['notCommitted', 'inputSent', 'mayHaveOccurred'].includes(String(receipt.delivery))
    && ['changed', 'unchanged', 'targetClosed', 'unavailable'].includes(String(receipt.evidence))
    && ['notRequested', 'matched', 'pending'].includes(String(receipt.condition))
    && typeof receipt.observationConsumed === 'boolean'
    ? receipt : null;
}

/** Tool completion and input delivery do not establish the requested outcome.
 * Only an explicit matched-condition receipt earns a success indicator. */
export function interactionPresentation(tool: ToolInput, parentRunActive = true): InteractionPresentation | null {
  if (!isInteractionTool(tool.toolName)) return null;
  if (isPendingToolCallStatus(tool.status)) return parentRunActive ? presentations.running : presentations.pending;
  const failed = Boolean(tool.isError) || isUnsuccessfulToolCallStatus(tool.status);
  const records = envelopes(tool.artifacts);
  const receipt = records.map(value => canonicalReceipt(value.interaction)).find(Boolean);
  if (receipt) {
    if (!failed && receipt.condition === 'matched') return presentations.matched;
    if (receipt.delivery === 'mayHaveOccurred') return presentations.uncertain;
    if (receipt.condition === 'pending' || (receipt.evidence === 'unavailable' && receipt.delivery === 'inputSent')) return presentations.pending;
    if (failed) return receipt.delivery === 'notCommitted' ? presentations.notDispatched : presentations.error;
    if (receipt.evidence === 'changed') return presentations.changed;
    if (receipt.evidence === 'unchanged') return presentations.unchanged;
    if (receipt.evidence === 'targetClosed') return presentations.targetClosed;
    if (receipt.delivery === 'notCommitted') return presentations.notDispatched;
    return presentations.pending;
  }

  // Older browser and desktop histories predate the shared interaction receipt.
  if (records.some(value => value.kind === 'browserActionReceipt' && value.status === 'uncertain'
    || value.deliveryStatus === 'uncertain')) return presentations.uncertain;
  if (records.some(value => value.kind === 'browserActionReceipt' && value.status === 'not_dispatched'
    || value.deliveryStatus === 'not_committed')) return presentations.notDispatched;
  if (records.some(value => value.kind === 'browserOpenReceipt' && value.observationPending === true
    || typeof value.observationError === 'string' && value.observationError.length > 0
    || value.effect === 'unverifiable' || value.effect === 'delivered_unverified')) return presentations.pending;
  if (failed) return tool.status === 'cancelled' ? presentations.cancelled
    : tool.status === 'declined' ? presentations.declined : presentations.error;

  const activity = record(records.find(value => value.kind === 'browserActivity')?.activity);
  const activityRecord = record(activity?.record);
  if (activityRecord) {
    const events = [...(Array.isArray(activity?.events) ? activity.events : []), ...(tool.activityEvents ?? [])]
      .map(record).filter((event): event is RecordValue => Boolean(event && event.activityId === activityRecord.activityId));
    const states = events.map(event => record(event.payload)?.state).filter((state): state is string => typeof state === 'string');
    const state = states[states.length - 1] ?? activityRecord.state;
    if (['failed', 'timed_out', 'cancelled', 'superseded'].includes(String(state))) return presentations.error;
    if (events.some(event => event.kind === 'browser_observation' && record(event.payload)?.matched === true)) return presentations.matched;
    return state === 'completed' ? presentations.done : presentations.pending;
  }
  if (records.some(value => value.effect === 'observed_unchanged' || value.changed === false)) return presentations.unchanged;
  if (records.some(value => value.effect === 'observed_change' || value.changed === true)) return presentations.changed;
  if (records.some(value => value.effect === 'window_closed' || value.kind === 'computerWindowClosure')) return presentations.targetClosed;
  if (records.some(value => ['computerObservation', 'browserObservation', 'browserEvidenceCapture'].includes(String(value.kind)))) return presentations.observed;
  let action = '';
  try { action = String(JSON.parse(tool.arguments ?? '{}').action ?? ''); } catch { /* Streaming/legacy arguments can be incomplete. */ }
  const readOnly = tool.capabilities?.readOnly || tool.toolName === 'computer_observe' || tool.toolName === 'browser_evidence_capture'
    || tool.toolName === 'browser_session' && ['observe', 'list_sessions', 'list_tabs'].includes(action);
  return readOnly ? presentations.done : presentations.pending;
}

/** Return stable objects so every step can share one external-store subscription. */
export function summarizeInteractions(items: InteractionPresentation[]): InteractionPresentation {
  return items.find(item => item.pending)
    ?? items.find(item => item.tone === 'danger')
    ?? items.find(item => item === presentations.uncertain)
    ?? items.find(item => item.tone === 'warning')
    ?? items[items.length - 1] ?? presentations.done;
}
