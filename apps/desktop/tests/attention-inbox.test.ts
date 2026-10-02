import { AttentionInbox } from '../src/lib/attentionInbox';
import { InteractionStore } from '../src/lib/interactionStore';
import type { ApprovalRequest, InteractionRequest } from '../src/types/conversation';

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const interactions = new InteractionStore(null);
const inbox = new AttentionInbox(interactions);
const approval: ApprovalRequest = {
  id: 'approval-alpha', toolName: 'run_shell', permissionKey: 'permission',
  targetKind: 'shell', targetValue: 'workspace', argumentsPreview: '{}', riskLevel: 'high',
  reason: 'Run the requested command', expiresAt: '2026-10-02T12:01:00Z',
};
const question: InteractionRequest = {
  schemaVersion: 1, interactionId: 'question-beta', conversationId: 'beta', turnId: 'turn-beta',
  kind: 'user_input', title: 'Choose a scope', questions: [], required: true, status: 'pending',
  riskPriority: 100, queueSequence: 1, createdAt: '2026-10-02T12:00:00Z', updatedAt: '2026-10-02T12:00:00Z', resumeToken: 'resume',
};

inbox.replaceApprovals('alpha', 'run-alpha', [approval]);
const approvalSnapshot = inbox.getSnapshot();
const approvalQueue = inbox.getApprovals('alpha');
let changes = 0;
inbox.subscribe(() => { changes++; });
inbox.replaceApprovals('alpha', 'run-alpha', approvalQueue);
assert(changes === 0 && inbox.getSnapshot() === approvalSnapshot, 'unrelated stream output must preserve the attention snapshot');
interactions.replaceRequests(null, [question]);
assert(inbox.getSnapshot().length === 2, 'background approval and question share one discovery surface');
assert(inbox.getApprovals('alpha') === approvalQueue, 'question changes must not invalidate the approval selector');
const beforePoll = inbox.getSnapshot();
interactions.replaceRequests(null, [{ ...question }]);
assert(inbox.getSnapshot() === beforePoll, 'unchanged hydration preserves requests and selectors');

const staleRevision = inbox.getApprovalRevision();
inbox.replaceApprovals('alpha', 'run-alpha', []);
assert(!inbox.replaceApprovalSnapshot([{ conversationId: 'alpha', runId: 'run-alpha', request: approval }], staleRevision), 'late snapshots must not resurrect a resolved approval');
assert(inbox.getApprovals('alpha').length === 0, 'resolved request remains absent');
assert(inbox.replaceApprovalSnapshot([{ conversationId: 'alpha', runId: 'run-alpha', request: approval }], inbox.getApprovalRevision()), 'authoritative pending prompts restore without an open conversation');
assert(inbox.getSnapshot().some(item => item.id === 'approval:approval-alpha'), 'restored prompt is discoverable');
const restored = inbox.getApprovals('alpha');
inbox.replaceApprovalSnapshot([{ conversationId: 'alpha', runId: 'run-alpha', request: { ...approval } }], inbox.getApprovalRevision());
assert(inbox.getApprovals('alpha') === restored, 'equal host snapshots preserve the narrow selector');
inbox.replaceApprovalSnapshot([], inbox.getApprovalRevision());
assert(inbox.getSnapshot().length === 1 && inbox.getSnapshot()[0].id === 'interaction:question-beta', 'host removal clears only approvals');
interactions.upsertRequest({ ...question, status: 'cancelled' });
assert(inbox.getSnapshot().length === 0, 'terminal interactions leave no stale count');
console.log('attention inbox contracts passed');
