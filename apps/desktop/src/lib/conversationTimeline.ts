import type {
  AgentTaskRun, ConversationMessage, ConversationTimelineCursor, ConversationTimelineDetails,
  ConversationTimelineEntry, ConversationTimelinePage, ConversationTimelineRange, ConversationTurn,
} from '../types/conversation';
import { estimateJsonBytes } from './boundedConversationCache';

export interface TimelineEntry extends ConversationTimelineEntry {
  detailsLoaded?: boolean;
  detailsLoading?: boolean;
  detailError?: string | null;
  detailRange?: ConversationTimelineRange;
  detailsCommittedAt?: number;
}

export interface TimelineSnapshot {
  messages: ConversationMessage[];
  turns: ConversationTurn[];
  taskRuns: AgentTaskRun[];
  entries: TimelineEntry[];
  oldestCursor: ConversationTimelineCursor | null;
  newestCursor: ConversationTimelineCursor | null;
  hasMoreBefore: boolean;
  hasMoreAfter: boolean;
  loadingOlder: boolean;
}

export interface TimelinePageRequest {
  before?: ConversationTimelineCursor | null;
  after?: ConversationTimelineCursor | null;
  anchorMessageId?: string | null;
  limit?: number;
}

interface TimelinePort {
  page(conversationId: string, request?: TimelinePageRequest): Promise<ConversationTimelinePage>;
  details(conversationId: string, anchorId: string): Promise<ConversationTimelineDetails>;
  mergeLocalMessages(previous: ConversationMessage[], next: ConversationMessage[]): ConversationMessage[];
  protectedIds(): string[];
}

type Update<T> = T | ((previous: T) => T);
type TimelineMutation = { messages?: Update<ConversationMessage[]>; turns?: Update<ConversationTurn[]>; taskRuns?: Update<AgentTaskRun[]> };
const EMPTY: TimelineSnapshot = { messages: [], turns: [], taskRuns: [], entries: [], oldestCursor: null, newestCursor: null, hasMoreBefore: false, hasMoreAfter: false, loadingOlder: false };
// SQLite's BINARY id ordering is independent of the WebView's locale.
const compare = (left: ConversationTimelineCursor, right: ConversationTimelineCursor) => left.sortOrder - right.sortOrder || (left.messageId < right.messageId ? -1 : left.messageId > right.messageId ? 1 : 0);
const key = (message: ConversationMessage): ConversationTimelineCursor => ({ sortOrder: message.sortOrder, messageId: message.id });
const inRange = (cursor: ConversationTimelineCursor, range: ConversationTimelineRange) => compare(cursor, range.from) >= 0 && (!range.before || compare(cursor, range.before) < 0);
const apply = <T,>(previous: T, update: Update<T> | undefined): T => update === undefined ? previous : typeof update === 'function' ? (update as (value: T) => T)(previous) : update;

/** Owns display pages, optimistic edits, lazy details and their cancellation.
 * It never changes the durable conversation or the running agent's context. */
export class ConversationTimeline {
  private readonly snapshots = new Map<string, TimelineSnapshot>();
  private readonly generations = new Map<string, number>();
  private readonly tailRequests = new Map<string, symbol>();
  private readonly detailRequests = new Map<string, Promise<void>>();
  private readonly sizes = new Map<string, number>();
  private readonly listeners = new Set<() => void>();
  private detailCommit = 0;

  constructor(private readonly port: TimelinePort) {}

  get = (conversationId?: string | null): TimelineSnapshot => conversationId ? this.snapshots.get(conversationId) ?? EMPTY : EMPTY;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };

  mutate(conversationId: string, mutation: TimelineMutation): void {
    const previous = this.get(conversationId);
    this.put(conversationId, { ...previous, messages: apply(previous.messages, mutation.messages), turns: apply(previous.turns, mutation.turns), taskRuns: apply(previous.taskRuns, mutation.taskRuns) });
  }

  invalidate(conversationId: string): void {
    this.fence(conversationId);
    const previous = this.get(conversationId);
    this.put(conversationId, { ...previous, newestCursor: null, loadingOlder: false, entries: previous.entries.map(entry => ({ ...entry, detailsLoaded: false, detailsLoading: false, detailError: null })) }, false);
  }

  fence(conversationId: string): void {
    this.generations.set(conversationId, this.generation(conversationId) + 1);
    this.tailRequests.delete(conversationId);
    const previous = this.snapshots.get(conversationId);
    if (previous && (previous.loadingOlder || previous.entries.some(entry => entry.detailsLoading))) {
      // Abandoned requests cannot run their generation-guarded cleanup. The
      // cancellation owner must release their loading state now, while a late
      // response remains unable to clear a newer request's loading indicator.
      this.put(conversationId, {
        ...previous,
        loadingOlder: false,
        entries: previous.entries.map(entry => entry.detailsLoading ? { ...entry, detailsLoading: false } : entry),
      }, false);
    }
  }

  clear(conversationId: string): void {
    this.generations.set(conversationId, this.generation(conversationId) + 1);
    this.tailRequests.delete(conversationId);
    this.snapshots.delete(conversationId);
    this.sizes.delete(conversationId);
    this.notify();
  }

  clearAll(): void { for (const id of new Set([...this.snapshots.keys(), ...this.generations.keys()])) this.clear(id); }

  async openTail(conversationId: string, anchorMessageId?: string | null): Promise<ConversationTimelinePage | null> {
    this.fence(conversationId);
    const generation = this.generation(conversationId);
    return this.readAndMergePage(conversationId, { anchorMessageId }, 'replace', () => this.generation(conversationId) === generation);
  }

  async refreshTail(conversationId: string): Promise<ConversationTimelinePage | null> {
    const previous = this.get(conversationId);
    if (!previous.newestCursor || previous.hasMoreAfter) return this.openTail(conversationId);
    const generation = this.generation(conversationId);
    const token = Symbol();
    this.tailRequests.set(conversationId, token);
    try {
      const page = await this.readAndMergePage(conversationId, { after: previous.newestCursor }, 'suffix', () => this.generation(conversationId) === generation && this.tailRequests.get(conversationId) === token);
      if (!page) return null;
      // Retry/compaction may have retired the old cursor. Re-open a bounded tail,
      // never fall back to fetching all messages from the old endpoint.
      if (!page.range) return this.openTail(conversationId);
      return page;
    } finally {
      if (this.tailRequests.get(conversationId) === token) this.tailRequests.delete(conversationId);
    }
  }

  async loadBefore(conversationId: string): Promise<void> {
    const previous = this.get(conversationId);
    if (!previous.oldestCursor || !previous.hasMoreBefore || previous.loadingOlder) return;
    const generation = this.generation(conversationId);
    this.put(conversationId, { ...previous, loadingOlder: true }, false);
    try {
      await this.readAndMergePage(conversationId, { before: previous.oldestCursor }, 'older', () => this.generation(conversationId) === generation);
    } finally {
      if (this.generation(conversationId) === generation) this.put(conversationId, { ...this.get(conversationId), loadingOlder: false }, false);
    }
  }

  loadDetails(conversationId: string, anchorId: string): Promise<void> {
    const generation = this.generation(conversationId);
    const requestKey = `${conversationId}:${anchorId}:${generation}`;
    const existing = this.detailRequests.get(requestKey);
    if (existing) return existing;
    const entry = this.get(conversationId).entries.find(entry => entry.anchor.messageId === anchorId);
    if (!entry || entry.detailsLoaded) return Promise.resolve();
    this.updateEntry(conversationId, anchorId, { detailsLoading: true, detailError: null });
    const request = this.port.details(conversationId, anchorId).then(details => {
      const current = this.get(conversationId);
      const latest = current.entries.find(entry => entry.anchor.messageId === anchorId);
      if (this.generation(conversationId) !== generation || !latest) return;
      // A completion can supersede the detail request. Keep the fresh summary
      // and let an explicit retry read that version, rather than restore old data.
      if (latest.detailRevision !== entry.detailRevision && latest.detailRevision !== details.detailRevision) return;
      this.put(conversationId, {
        ...current,
        messages: this.mergeMessages(current.messages, details.messages, details.range),
        turns: this.mergeTurns(current.turns, details.turns, new Set(details.turns.map(turn => turn.id))),
        entries: current.entries.map(item => item.anchor.messageId === anchorId ? { ...item, detailRevision: details.detailRevision, detailsLoaded: true, detailsLoading: false, detailError: null, detailRange: details.range, detailsCommittedAt: ++this.detailCommit } : item),
      });
    }).catch(error => {
      if (this.generation(conversationId) === generation) this.updateEntry(conversationId, anchorId, { detailError: String(error) });
    }).finally(() => {
      if (this.detailRequests.get(requestKey) === request) this.detailRequests.delete(requestKey);
      if (this.generation(conversationId) === generation) this.updateEntry(conversationId, anchorId, { detailsLoading: false });
    });
    this.detailRequests.set(requestKey, request);
    return request;
  }

  private generation(id: string): number { return this.generations.get(id) ?? 0; }

  private async readAndMergePage(id: string, request: TimelinePageRequest, mode: 'replace' | 'older' | 'suffix', isCurrent: () => boolean): Promise<ConversationTimelinePage | null> {
    const startedAt = this.detailCommit;
    // A page can start before a newer explicit detail read and arrive afterward.
    // Revision tokens are opaque: reread that same bounded range once instead of
    // guessing their order. Continued competition keeps the committed detail.
    for (let attempt = 0; attempt < 2; attempt++) {
      const page = await this.port.page(id, request);
      if (!isCurrent()) return null;
      if (!page || !Array.isArray(page.messages) || !Array.isArray(page.turns) || !Array.isArray(page.entries)) throw new Error('Invalid conversation timeline page');
      const conflicts = this.get(id).entries.some(entry => entry.detailsLoaded && (entry.detailsCommittedAt ?? 0) > startedAt
        && (mode === 'replace' || !page.range || inRange(entry.anchor, page.range))
        && !page.entries.some(next => next.anchor.messageId === entry.anchor.messageId && next.detailRevision === entry.detailRevision));
      if (!conflicts) {
        // Commit in this continuation: an await between the conflict check and
        // merge would let another detail completion enter that gap.
        if (page.range || mode !== 'suffix') this.mergePage(id, page, mode);
        return page;
      }
    }
    return null;
  }

  private mergePage(id: string, page: ConversationTimelinePage, mode: 'replace' | 'older' | 'suffix'): void {
    if (!page || !Array.isArray(page.messages) || !Array.isArray(page.turns) || !Array.isArray(page.entries)) throw new Error('Invalid conversation timeline page');
    const previous = this.get(id);
    const retained = previous.entries.filter(entry => entry.detailsLoaded && entry.detailRange
      && page.entries.some(next => next.anchor.messageId === entry.anchor.messageId && next.detailRevision === entry.detailRevision));
    const preserveRanges = retained.flatMap(entry => entry.detailRange ? [entry.detailRange] : []);
    const preservedTurns = new Set(retained.flatMap(entry => entry.turnId ? [entry.turnId] : []));
    const entries = new Map((mode === 'replace' ? [] : previous.entries.filter(entry => !page.range || !inRange(entry.anchor, page.range))).map(entry => [entry.anchor.messageId, entry]));
    for (const entry of page.entries) entries.set(entry.anchor.messageId, retained.find(old => old.anchor.messageId === entry.anchor.messageId) ?? entry);
    const removedTurns = new Set(previous.entries.filter(entry => page.range && inRange(entry.anchor, page.range) && !preservedTurns.has(entry.turnId ?? '')).flatMap(entry => entry.turnId ? [entry.turnId] : []));
    const turns = this.mergeTurns(mode === 'replace' ? previous.turns.filter(turn => preservedTurns.has(turn.id)) : previous.turns, page.turns.filter(turn => !preservedTurns.has(turn.id)), removedTurns);
    this.put(id, {
      messages: this.mergeMessages(previous.messages, page.messages, page.range, preserveRanges, mode === 'replace'),
      turns,
      taskRuns: page.taskRuns,
      entries: [...entries.values()].sort((left,right) => compare(left.anchor,right.anchor)),
      oldestCursor: mode === 'suffix' ? previous.oldestCursor ?? page.oldestCursor : mode === 'replace' ? page.oldestCursor : page.oldestCursor ?? previous.oldestCursor,
      newestCursor: mode === 'older' ? previous.newestCursor : page.newestCursor,
      hasMoreBefore: mode === 'suffix' ? previous.hasMoreBefore : page.hasMoreBefore,
      hasMoreAfter: mode === 'older' ? previous.hasMoreAfter : page.hasMoreAfter,
      loadingOlder: previous.loadingOlder,
    });
  }

  private mergeMessages(previous: ConversationMessage[], incoming: ConversationMessage[], range: ConversationTimelineRange | null, preserved: ConversationTimelineRange[] = [], replace = false): ConversationMessage[] {
    const keepDetail = (message: ConversationMessage) => preserved.some(range => inRange(key(message), range));
    const values = new Map(previous.filter(message => keepDetail(message) || (!replace && (!range || !inRange(key(message), range)))).map(message => [message.id, message]));
    for (const message of incoming) if (!keepDetail(message)) values.set(message.id, message);
    const next = [...values.values()].sort((left,right) => compare(key(left),key(right)));
    return this.port.mergeLocalMessages(previous, next);
  }

  private mergeTurns(previous: ConversationTurn[], incoming: ConversationTurn[], removed: Set<string>): ConversationTurn[] {
    const values = new Map(previous.filter(turn => !removed.has(turn.id)).map(turn => [turn.id,turn]));
    for (const turn of incoming) values.set(turn.id,turn);
    return [...values.values()];
  }

  private updateEntry(id: string, anchorId: string, patch: Partial<TimelineEntry>): void {
    const previous = this.get(id);
    this.put(id, { ...previous, entries: previous.entries.map(entry => entry.anchor.messageId === anchorId ? { ...entry, ...patch } : entry) }, false);
  }

  private put(id: string, snapshot: TimelineSnapshot, dataChanged = true): void {
    if (dataChanged && snapshot.turns.length > 1) {
      const positions = new Map(snapshot.messages.map((message,index) => [message.id,index]));
      snapshot = { ...snapshot, turns: [...snapshot.turns].sort((left,right) => {
        const a = positions.get(left.userMessageId), b = positions.get(right.userMessageId);
        return a != null && b != null ? a-b : 0;
      }) };
    }
    this.snapshots.delete(id);
    this.snapshots.set(id,snapshot);
    if (dataChanged) this.sizes.set(id, estimateJsonBytes(snapshot));
    const protectedIds = new Set([id,...this.port.protectedIds()]);
    let bytes = [...this.sizes.values()].reduce((total,size) => total+size,0);
    for (const candidate of this.snapshots.keys()) {
      if (this.snapshots.size <= 8 && bytes <= 96*1024*1024) break;
      if (protectedIds.has(candidate)) continue;
      bytes -= this.sizes.get(candidate) ?? 0;
      this.snapshots.delete(candidate);
      this.sizes.delete(candidate);
      this.generations.set(candidate, this.generation(candidate)+1);
    }
    this.notify();
  }

  private notify(): void { for (const listener of this.listeners) listener(); }
}
