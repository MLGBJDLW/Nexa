export type BrowserActionPhase =
  | 'moving' | 'committing' | 'waiting'
  | 'verified' | 'observedUnchanged' | 'observedPending' | 'failed' | 'interrupted';

export interface BrowserActionEntry {
  action: string;
  phase: BrowserActionPhase;
  tabId: string;
  operationId: string;
  callId: string;
  sequence: number;
}

export interface BrowserActionProjection {
  entries: BrowserActionEntry[];
  retiredThroughSequence: number;
  latestSequence: number;
}

export function emptyBrowserActionProjection(): BrowserActionProjection {
  return { entries: [], retiredThroughSequence: 0, latestSequence: 0 };
}

const PHASE_ORDER: Record<BrowserActionPhase, number> = {
  moving: 0, waiting: 0, committing: 1,
  verified: 2, observedUnchanged: 2, observedPending: 2, failed: 2, interrupted: 2,
};

export function browserActionInProgress(phase: BrowserActionPhase): boolean {
  return PHASE_ORDER[phase] < 2;
}

/** Project one entry per host invocation, independent of provider call ID reuse.
 * A terminal receipt cannot be reverted by a late cursor/progress event.
 */
export function projectBrowserAction(
  previous: BrowserActionProjection,
  payload: Record<string, unknown>,
  sequence: unknown,
): BrowserActionProjection {
  const { action, phase, tabId, operationId, callId } = payload;
  if (typeof action !== 'string' || !action || typeof tabId !== 'string' || !tabId
    || typeof operationId !== 'string' || !operationId
    || typeof callId !== 'string' || !callId || typeof phase !== 'string'
    || !Object.prototype.hasOwnProperty.call(PHASE_ORDER, phase)
    || typeof sequence !== 'number' || !Number.isSafeInteger(sequence) || sequence <= 0) return previous;
  const next: BrowserActionEntry = { action, phase: phase as BrowserActionPhase, tabId, operationId, callId, sequence };
  const index = previous.entries.findIndex(entry => entry.operationId === operationId && entry.tabId === tabId);
  if (index < 0) {
    // Native event order lets one bounded watermark replace an unbounded set
    // of tombstones after old calls leave the six-row presentation window.
    if (sequence <= previous.retiredThroughSequence) return previous;
    const entries = [...previous.entries, next];
    const retired = entries.length > 6 ? entries.shift() : undefined;
    return {
      entries,
      latestSequence: Math.max(previous.latestSequence, sequence),
      retiredThroughSequence: Math.max(previous.retiredThroughSequence, retired?.sequence ?? 0),
    };
  }
  const current = previous.entries[index];
  if (sequence <= current.sequence || current.action !== action || current.callId !== callId || !browserActionInProgress(current.phase)
    || PHASE_ORDER[next.phase] <= PHASE_ORDER[current.phase]) return previous;
  return {
    ...previous,
    latestSequence: Math.max(previous.latestSequence, sequence),
    entries: previous.entries.map((entry, position) => position === index ? next : entry),
  };
}

export function interruptBrowserActions(previous: BrowserActionProjection, sequence?: unknown): BrowserActionProjection {
  const cutoff = typeof sequence === 'number' && Number.isSafeInteger(sequence)
    ? sequence : previous.latestSequence;
  const interrupts = (entry: BrowserActionEntry) => entry.sequence <= cutoff && browserActionInProgress(entry.phase);
  if (cutoff <= previous.retiredThroughSequence && !previous.entries.some(interrupts)) return previous;
  return {
    latestSequence: Math.max(previous.latestSequence, cutoff),
    retiredThroughSequence: Math.max(previous.retiredThroughSequence, cutoff),
    entries: previous.entries.map(entry => interrupts(entry)
      ? { ...entry, phase: 'interrupted' } : entry),
  };
}
