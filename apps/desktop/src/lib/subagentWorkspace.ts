import type { ActivityEvent, AgentSubtaskRun, ToolRunItem } from '../types/conversation';
import { createDefaultState } from './streaming/state';
import { appendReplyTraceEvent, appendThinkingTraceEvent, applyStreamBlockDelta, applyStreamBlockSnapshot } from './streaming/blockProjection';
import { appendStatusTraceEvent, applyStreamResetProjection, finishProjectedTools } from './streaming/terminalProjection';
import { applyToolRunEvent } from './streaming/toolProjection';

export interface SubagentWorkspacePage {
  agentId: string | null;
  journal: { events: ActivityEvent[]; cursor: number; hasMore: boolean; historyTruncated: boolean; privacyRevision: string } | null;
  legacyRun: AgentSubtaskRun | null;
  status: string;
  canControl: boolean;
  privacyRevision: string;
}
export function subagentRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
}
const record = subagentRecord;
function text(value: unknown): string { return typeof value === 'string' ? value : ''; }

/** Owns one child's projection. It never enters the parent's stream store. */
export class SubagentJournalProjection {
  readonly state = createDefaultState();
  task = '';
  role = '';
  result = '';
  cursor = 0;
  private canonical = new Set<string>();
  private approvals = new Map<string, string>();

  append(events: ActivityEvent[]) {
    for (const item of events) {
      if (item.seq <= this.cursor) continue;
      this.cursor = item.seq;
      const payload = record(item.payload);
      const detail = record(payload.detail);
      const kind = text(payload.subagentEvent);
      if (kind === 'spawned') { this.task = text(detail.task); this.role = text(detail.role); }
      else if (kind === 'stream') {
        const event = record(detail.event);
        const channel = event.channel === 'thinking' ? 'thinking' : 'answer';
        if (event.type === 'streamBlockDelta' || event.type === 'streamBlockSnapshot') {
          if (event.legacy === true && this.canonical.has(channel)) continue;
          if (event.legacy !== true && !this.canonical.has(channel)) {
            // Some adapters dual-emit their compatibility text. Canonical blocks
            // are authoritative, including when a snapshot arrives after text.
            const traceKind = channel === 'thinking' ? 'thinking' : 'reply';
            this.state.traceEvents = this.state.traceEvents.filter(entry => entry.kind !== traceKind || (!!entry.blockId && !entry.blockId.startsWith('legacy-worker-')));
            this.canonical.add(channel);
          }
          if (event.type === 'streamBlockDelta') applyStreamBlockDelta(this.state, channel, text(event.blockId), Number(event.offset), text(event.delta));
          else applyStreamBlockSnapshot(this.state, channel, text(event.blockId), text(event.text));
        } else if (event.type === 'streamReset') {
          applyStreamResetProjection(this.state, text(event.reason), { discardSample: event.discard_sample === true || event.discardSample === true });
        } else if (event.type === 'autoCompacted') {
          appendStatusTraceEvent(this.state, text(event.summary));
        } else if (event.type === 'approvalRequested') {
          const request = record(event.request);
          appendStatusTraceEvent(this.state, text(request.toolName), 'muted', 'user', 'status', 'subagentApproval');
          const entry = this.state.traceEvents[this.state.traceEvents.length - 1];
          if (entry && typeof request.id === 'string') this.approvals.set(request.id, entry.id);
        } else if (event.type === 'approvalResolved') {
          const entryId = this.approvals.get(text(event.requestId));
          if (entryId) this.state.traceEvents = this.state.traceEvents.filter(entry => entry.id !== entryId);
          this.approvals.delete(text(event.requestId));
        } else if (event.type === 'planUpdated') {
          const plan = record(event.plan);
          for (const step of Array.isArray(plan.steps) ? plan.steps : []) {
            const row = record(step);
            appendStatusTraceEvent(this.state, `${text(row.status)} · ${text(row.title)}`);
          }
        }
      } else if (kind === 'thinkingDelta' && !this.canonical.has('thinking')) {
        appendThinkingTraceEvent(this.state, text(detail.delta));
      } else if (kind === 'outputDelta' && !this.canonical.has('answer')) {
        appendReplyTraceEvent(this.state, text(detail.delta));
      } else if (detail.run) {
        const run = record(detail.run);
        if (typeof run.callId === 'string' && typeof run.toolName === 'string') applyToolRunEvent(this.state, run as unknown as ToolRunItem);
      } else if (kind === 'inputQueued' || kind === 'inputApplied') {
        appendStatusTraceEvent(this.state, text(detail.content), 'muted', 'user', 'status', kind);
      } else if (kind === 'completed' || kind === 'failed' || kind === 'cancelled') {
        const result = record(detail.result);
        this.result = text(result.result) || text(result.content) || text(detail.result);
        finishProjectedTools(this.state, kind === 'completed' ? 'done' : kind === 'cancelled' ? 'cancelled' : 'error', text(detail.errorMessage));
        if (detail.errorMessage) appendStatusTraceEvent(this.state, text(detail.errorMessage), 'error');
      } else if (detail.content) {
        appendStatusTraceEvent(this.state, text(detail.content));
      }
    }
  }
}
