import type { ApprovalRequest, InteractionRequest } from '../types/conversation';
import { InteractionStore, interactionStore } from './interactionStore';

/** A read model only. The approval runtime and interaction ledger own responses. */
export interface AttentionItem {
  id: string;
  conversationId: string;
  runId: string | null;
  kind: 'approval' | 'interaction';
  title: string;
  description: string;
  expiresAt: string | null;
  request: ApprovalRequest | InteractionRequest;
}

interface ApprovalProjection {
  runId: string | null;
  requests: readonly ApprovalRequest[];
  items: AttentionItem[];
}

const EMPTY_ITEMS: readonly AttentionItem[] = [];
const EMPTY_APPROVALS: readonly ApprovalRequest[] = [];

export interface PendingApprovalSnapshot {
  conversationId: string;
  runId: string;
  request: ApprovalRequest;
}

export class AttentionInbox {
  private readonly approvals = new Map<string, ApprovalProjection>();
  private readonly listeners = new Set<() => void>();
  private snapshot: readonly AttentionItem[] = EMPTY_ITEMS;
  private interactions: readonly InteractionRequest[] = [];
  private approvalRevision = 0;

  constructor(private readonly interactionSource: InteractionStore) {
    this.refreshInteractions();
    interactionSource.subscribe(() => this.refreshInteractions());
  }

  getSnapshot = (): readonly AttentionItem[] => this.snapshot;
  getApprovalRevision = (): number => this.approvalRevision;
  getApprovals = (conversationId: string): readonly ApprovalRequest[] => this.approvals.get(conversationId)?.requests ?? EMPTY_APPROVALS;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  replaceApprovals(conversationId: string, runId: string | null, requests: readonly ApprovalRequest[]): void {
    const previous = this.approvals.get(conversationId);
    if (requests.length === 0) {
      if (!this.approvals.delete(conversationId)) return;
    } else {
      if (previous?.runId === runId && previous.requests === requests) return;
      this.approvals.set(conversationId, {
        runId,
        requests,
        items: requests.map(request => ({
          id: `approval:${request.id}`,
          conversationId,
          runId,
          kind: 'approval',
          title: request.toolName,
          description: request.reason,
          expiresAt: request.expiresAt ?? null,
          request,
        })),
      });
    }
    this.approvalRevision += 1;
    this.publish();
  }

  /** A snapshot started before a live approval transition cannot resurrect it. */
  replaceApprovalSnapshot(entries: readonly PendingApprovalSnapshot[], startedAtRevision: number): boolean {
    if (startedAtRevision !== this.approvalRevision) return false;
    const next = new Map<string, { runId: string; requests: ApprovalRequest[] }>();
    for (const entry of entries) {
      const group = next.get(entry.conversationId) ?? { runId: entry.runId, requests: [] };
      // A conversation owns one active run. Reject a mixed snapshot instead of
      // presenting an approval under an unrelated run identity.
      if (group.runId !== entry.runId) return false;
      if (!group.requests.some(request => request.id === entry.request.id)) group.requests.push(entry.request);
      next.set(entry.conversationId, group);
    }
    for (const conversationId of this.approvals.keys()) {
      if (!next.has(conversationId)) this.replaceApprovals(conversationId, null, EMPTY_APPROVALS);
    }
    for (const [conversationId, group] of next) {
      const previous = this.approvals.get(conversationId);
      const requests = previous?.runId === group.runId && JSON.stringify(previous.requests) === JSON.stringify(group.requests)
        ? previous.requests : group.requests;
      this.replaceApprovals(conversationId, group.runId, requests);
    }
    return true;
  }

  private refreshInteractions(): void {
    const requests = this.interactionSource.queue();
    if (requests.length === this.interactions.length
      && requests.every((request, index) => request === this.interactions[index])) return;
    this.interactions = requests;
    this.publish();
  }

  private publish(): void {
    this.snapshot = [
      ...[...this.approvals.values()].flatMap(projection => projection.items),
      ...this.interactions.map((request): AttentionItem => ({
        id: `interaction:${request.interactionId}`,
        conversationId: request.conversationId,
        runId: null,
        kind: 'interaction',
        title: request.title,
        description: request.description ?? '',
        expiresAt: request.expiresAt ?? null,
        request,
      })),
    ];
    for (const listener of this.listeners) listener();
  }
}

export const attentionInbox = new AttentionInbox(interactionStore);
