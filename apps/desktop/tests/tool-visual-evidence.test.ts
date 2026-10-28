import { extractToolVisualEvidence, mergeCurrentTurnVisualEvidence, mergeToolArtifacts, retainMessageVisualEvidence } from '../src/lib/toolVisualEvidence';
import { createDefaultState } from '../src/lib/streaming/state';
import { applyToolRunEvent, resolveToolCallResult } from '../src/lib/streaming/toolProjection';
import { interactionOperation, interactionPresentation, isInteractionTool, summarizeInteractions } from '../src/lib/streaming/interactionPresentation';
import type { ConversationMessage, ToolRunItem } from '../src/types/conversation';

function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
const evidence = (base64 = 'aGVsbG8=') => ({ kind: 'toolVisualEvidence', persistence: 'currentTurnOnly', evidence: { name: 'capture.png', mimeType: 'image/png', base64 } });
const receipt = { data: { screenshotHash: 'capture' }, artifacts: { kind: 'computerObservation' }, toolOutput: { displayContent: 'Captured Editor' } };
const state = createDefaultState();
const completed: ToolRunItem = { callId: 'capture', toolName: 'computer_observe', status: 'completed', artifacts: receipt,
  owner: { id: 'desktop', name: 'Desktop', capability: 'capture', description: 'Capture window' }, renderKind: 'generic',
  capabilities: { inputStreaming: 'none', renderKind: 'generic', readOnly: true, destructive: false, concurrencySafe: true, interruptBehavior: 'block', resourceKeys: [] } };
applyToolRunEvent(state, completed);
applyToolRunEvent(state, { ...completed, artifacts: evidence() });
applyToolRunEvent(state, completed);
assert(extractToolVisualEvidence(state.toolCalls[0].artifacts)?.base64 === 'aGVsbG8=', 'reconciliation must retain the live screenshot');
const finished = resolveToolCallResult(state.toolCalls, 'capture', false, 'Captured', receipt).next;
assert(extractToolVisualEvidence(finished[0].artifacts), 'final tool result must retain live pixels');
assert((finished[0].artifacts as typeof receipt).data.screenshotHash === 'capture', 'durable metadata must remain available');
assert(!('visualEvidence' in receipt), 'never mutate a durable artifact');

// High resolution PNGs legitimately reach the six MiB UI limit. A repeated
// four-character regex group exhausts V8's stack at this exact production size.
const highResolution = 'A'.repeat(6 * 1024 * 1024);
assert(extractToolVisualEvidence(evidence(highResolution))?.base64.length === highResolution.length, 'largest accepted screenshot must not crash the chat renderer');
for (const invalid of ['', 'a===', 'abcd@abc', 'aGVsbG8', `${highResolution}AAAA`]) {
  assert(extractToolVisualEvidence(evidence(invalid)) === null, 'invalid or oversized images must be rejected');
}
assert(extractToolVisualEvidence({ ...evidence(), persistence: 'durable' }) === null, 'only explicitly transient screenshot payloads are accepted');
assert(mergeToolArtifacts(receipt, undefined) === receipt, 'missing updates preserve metadata');

const message: ConversationMessage = { id: 'm', conversationId: 'current', role: 'tool', content: 'Captured Editor', toolCallId: 'capture', toolCalls: [], artifacts: receipt, tokenCount: 0, createdAt: '2026-09-22T00:00:00Z', sortOrder: 1, thinking: null, imageAttachments: null };
const stored = [{ ...message, id: 'owning-user', role: 'user' as const, toolCallId: null, artifacts: null }, message];
const displayed = mergeCurrentTurnVisualEvidence(stored, finished, 'current');
assert(extractToolVisualEvidence(displayed[1].artifacts ?? undefined), 'Done handover must show live pixels on the saved tool result');
assert(stored[1] === message && stored[1].artifacts === receipt, 'display merge must not modify the backend snapshot');
assert(mergeCurrentTurnVisualEvidence(stored, finished, 'other') === stored, 'pixels must not cross conversations');
assert(mergeCurrentTurnVisualEvidence(stored, [], 'current') === stored, 'reloading without transient calls must not invent historical pixels');
const oldPixels = evidence('b2xkMQ==');
const newPixels = evidence('bmV3Mg==');
const oldUser: ConversationMessage = { ...message, id: 'old-user', role: 'user', toolCallId: null, artifacts: null };
const newUser: ConversationMessage = { ...oldUser, id: 'new-user' };
const oldTool = { ...message, id: 'old-result', artifacts: mergeToolArtifacts(receipt, oldPixels)! };
const newTool = { ...message, id: 'new-result' };
const repeatedIds = mergeCurrentTurnVisualEvidence([oldUser, oldTool, newUser, newTool], [{ callId: 'capture', artifacts: newPixels }], 'current');
assert(extractToolVisualEvidence(repeatedIds[1].artifacts ?? undefined)?.base64 === 'b2xkMQ==', 'a reused provider call ID must not replace the previous turn screenshot');
assert(extractToolVisualEvidence(repeatedIds[3].artifacts ?? undefined)?.base64 === 'bmV3Mg==', 'the current turn must receive its own screenshot');
const reloaded = retainMessageVisualEvidence(repeatedIds, [oldUser, { ...oldTool, artifacts: receipt }, newUser, newTool]);
assert(extractToolVisualEvidence(reloaded[1].artifacts ?? undefined)?.base64 === 'b2xkMQ==', 'backend refresh must retain old pixels by exact message identity');
assert(extractToolVisualEvidence(reloaded[3].artifacts ?? undefined)?.base64 === 'bmV3Mg==', 'backend refresh must retain new pixels by exact message identity');
const noCurrentResult = [oldUser, oldTool, newUser];
assert(mergeCurrentTurnVisualEvidence(noCurrentResult, [{ callId: 'capture', artifacts: newPixels }], 'current') === noCurrentResult, 'an unfinished hydration must not attach current pixels to an old result');
assert(mergeCurrentTurnVisualEvidence(repeatedIds, [{ callId: 'capture', artifacts: oldPixels }], 'current', 'missing-owner') === repeatedIds, 'a missing authoritative turn owner must fail closed');
const ownedOldTurn = mergeCurrentTurnVisualEvidence([oldUser, { ...oldTool, artifacts: receipt }, newUser, newTool], [{ callId: 'capture', artifacts: oldPixels }], 'current', 'old-user');
assert(extractToolVisualEvidence(ownedOldTurn[1].artifacts ?? undefined)?.base64 === 'b2xkMQ==', 'an authoritative turn owner must receive its own late capture');
assert(!extractToolVisualEvidence(ownedOldTurn[3].artifacts ?? undefined), 'late capture must not cross into the next user turn');
assert(retainMessageVisualEvidence(repeatedIds, [{ ...newTool, id: 'replacement-result' }])[0].artifacts === receipt, 'a different result message must not inherit pixels even when its call ID repeats');
const captures = Array.from({ length: 14 }, (_, index) => ({ ...message, id: `capture-${index}`, toolCallId: `call-${index}`, artifacts: mergeToolArtifacts(receipt, evidence())! }));
const boundedCount = retainMessageVisualEvidence(captures, captures.map(capture => ({ ...capture, artifacts: receipt })));
assert(boundedCount.filter(capture => extractToolVisualEvidence(capture.artifacts ?? undefined)).length === 12, 'retain at most the newest 12 images in an active conversation');
assert(!extractToolVisualEvidence(boundedCount[1].artifacts ?? undefined) && extractToolVisualEvidence(boundedCount[13].artifacts ?? undefined), 'image-count eviction must keep the newest results');
assert((boundedCount[0].artifacts as typeof receipt).data === receipt.data && (boundedCount[0].artifacts as typeof receipt).toolOutput === receipt.toolOutput, 'eviction must preserve durable result metadata');
const largeCaptures = captures.slice(0, 7).map(capture => ({ ...capture, artifacts: mergeToolArtifacts(receipt, evidence(highResolution))! }));
const boundedBytes = retainMessageVisualEvidence(largeCaptures, largeCaptures.map(capture => ({ ...capture, artifacts: receipt })));
assert(boundedBytes.filter(capture => extractToolVisualEvidence(capture.artifacts ?? undefined)).length === 5, 'six MiB images must fit within the 32 MiB per-conversation budget');
assert(!extractToolVisualEvidence(boundedBytes[1].artifacts ?? undefined) && extractToolVisualEvidence(boundedBytes[6].artifacts ?? undefined), 'byte-budget eviction must preserve newest captures');
assert(extractToolVisualEvidence(largeCaptures[0].artifacts ?? undefined), 'budgeting must not mutate caller-owned message snapshots');
const handedOver = mergeCurrentTurnVisualEvidence([oldUser, ...captures], [{ callId: 'call-13', artifacts: newPixels }], 'current');
assert(handedOver.filter(capture => extractToolVisualEvidence(capture.artifacts ?? undefined)).length === 12, 'the Done handover must enforce the same image budget');
const separateConversations = retainMessageVisualEvidence([], [...captures.slice(0, 12), ...captures.slice(0, 12).map(capture => ({ ...capture, id: `other-${capture.id}`, conversationId: 'other' }))]);
assert(separateConversations.filter(capture => extractToolVisualEvidence(capture.artifacts ?? undefined)).length === 24, 'each conversation owns its own image budget');
const rawEnvelope = { ...message, id: 'raw-capture', artifacts: { ...evidence(), receiptId: 'preserve-me' } };
const boundedEnvelope = retainMessageVisualEvidence([], [rawEnvelope, ...captures.slice(0, 12)]);
assert(!extractToolVisualEvidence(boundedEnvelope[0].artifacts ?? undefined) && (boundedEnvelope[0].artifacts as Record<string, unknown>).receiptId === 'preserve-me', 'raw transient envelopes must lose only pixels, retaining receipt metadata');
// Browser and desktop use the same outcome vocabulary, independently of pixels
// and of whether the dispatcher itself returned a completed tool call.
const action = { toolName: 'computer_control', status: 'done' as const };
const interaction = (delivery: string, evidence: string, condition = 'notRequested') => ({ delivery, evidence, condition, observationConsumed: true });
for (const toolName of ['computer_use', 'computer_observe', 'computer_control', 'desktop_automation', 'browser_session', 'browser_evidence_capture']) assert(isInteractionTool(toolName), `${toolName} shares the interaction trace`);
assert(!isInteractionTool('run_shell') && interactionPresentation({ ...action, toolName: 'run_shell' }) === null, 'unrelated tools retain their own presentation');
for (const wrap of [(value: object) => ({ data: { interaction: value } }), (value: object) => ({ toolOutput: { data: { interaction: value } } }), (value: object) => ({ interaction: value })]) {
  assert(interactionPresentation({ ...action, artifacts: wrap(interaction('inputSent', 'changed')) })?.tone === 'neutral', 'changed pixels do not establish a requested outcome');
  assert(interactionPresentation({ ...action, artifacts: wrap(interaction('inputSent', 'unchanged')) })?.labelKey === 'chat.interactionUnchanged', 'an unchanged observation is not a success or input failure');
  assert(interactionPresentation({ ...action, artifacts: wrap(interaction('inputSent', 'unavailable')) })?.labelKey === 'chat.interactionPending', 'input without a post-action observation needs verification');
  assert(interactionPresentation({ ...action, artifacts: wrap(interaction('notCommitted', 'unchanged', 'matched')) })?.tone === 'success', 'a verified condition may already hold without sending input');
  assert(interactionPresentation({ ...action, isError: true, artifacts: wrap(interaction('mayHaveOccurred', 'unavailable')) })?.labelKey === 'chat.interactionUncertain', 'a dispatch failure with possible effects must not invite an unsafe retry');
}
for (const [artifacts, label] of [
  [{ kind: 'browserOpenReceipt', opened: true, observationPending: true }, 'chat.interactionPending'],
  [{ kind: 'browserActionReceipt', status: 'uncertain' }, 'chat.interactionUncertain'],
  [{ kind: 'browserActionReceipt', status: 'not_dispatched' }, 'chat.interactionNotDispatched'],
  [{ data: { deliveryStatus: 'delivered', effect: 'delivered_unverified' } }, 'chat.interactionPending'],
] as const) assert(interactionPresentation({ ...action, toolName: 'browser_session', isError: true, artifacts })?.labelKey === label, 'legacy receipts preserve their delivery/observation distinction');
assert(interactionPresentation(action)?.tone !== 'success', 'an action without evidence never gets a green success indicator');
assert(interactionPresentation({ ...action, artifacts: { data: { interaction: interaction('invented', 'changed', 'matched') } } })?.tone !== 'success', 'malformed receipts cannot claim verified conditions');
const pendingPresentation = interactionPresentation({ ...action, status: 'running' })!;
const failedPresentation = interactionPresentation({ ...action, status: 'error' })!;
const matchedPresentation = interactionPresentation({ ...action, artifacts: { interaction: interaction('inputSent', 'changed', 'matched') } })!;
assert(summarizeInteractions([pendingPresentation, matchedPresentation]).pending, 'an earlier pending action keeps the whole group active');
assert(summarizeInteractions([failedPresentation, matchedPresentation]).tone === 'danger', 'a late earlier failure cannot hide behind a later successful condition');
assert(interactionPresentation({ ...action, status: 'running' }, false)?.pending === false, 'a settled parent cannot leave a historical spinner running');
const browserWait = (state: string, matched: boolean) => ({ kind: 'browserActivity', activity: {
  record: { activityId: 'wait-1', state },
  events: matched ? [{ activityId: 'wait-1', kind: 'browser_observation', payload: { matched: true } }] : [],
} });
assert(interactionPresentation({ ...action, toolName: 'browser_session', artifacts: browserWait('running', false) })?.labelKey === 'chat.interactionPending', 'a returned browser wait handle does not mean its condition matched');
assert(interactionPresentation({ ...action, toolName: 'browser_session', artifacts: browserWait('completed', true) })?.tone === 'success', 'browser wait evidence can establish the requested condition');
assert(interactionPresentation({ ...action, toolName: 'browser_session', artifacts: browserWait('timed_out', false) })?.tone === 'danger', 'a timed-out browser wait cannot remain a completed green call');
const unifiedAction = { ...action, toolName: 'computer_use' };
const publicWindowArgs = JSON.stringify({ target: { kind: 'window', window_id: 42 }, action: 'type', text: 'never show this in a title' });
const auditedWindowArgs = JSON.stringify({ targetKind: 'window', action: 'type_text', text: { redacted: true } });
assert(JSON.stringify(interactionOperation('computer_use', publicWindowArgs)) === JSON.stringify(interactionOperation('computer_use', auditedWindowArgs)), 'public and persisted unified calls retain the same backend action label');
assert(interactionOperation('computer_use', publicWindowArgs)?.action === 'type_text', 'native aliases display the actual backend operation without typed text');
assert(interactionOperation('computer_use', '{"targetKind":"browser","action":"click"}')?.targetKind === 'browser', 'audited browser targets retain their browser icon identity');
assert(interactionOperation('computer_use', '{"target":{"kind":"window"},"action":"not a routing action"}') === null, 'unrecognized routing fields never become a title');
assert(interactionPresentation({ ...unifiedAction, arguments: auditedWindowArgs })?.labelKey === 'chat.interactionPending', 'a completed unified input without evidence is still unverified');
assert(interactionPresentation({ ...unifiedAction, arguments: publicWindowArgs, artifacts: { interaction: interaction('inputSent', 'unavailable') } })?.labelKey === 'chat.interactionPending', 'unified native input retains the shared pending receipt');
assert(interactionPresentation({ ...unifiedAction, artifacts: browserWait('completed', true) })?.tone === 'success', 'unified browser calls preserve explicit matched-condition evidence');
assert(interactionPresentation({ ...unifiedAction, arguments: '{"target":{"kind":"browser"},"action":"observe"}', capabilities: { ...completed.capabilities!, readOnly: false } })?.labelKey === 'chat.interactionPending', 'host capabilities override an action name when classifying a unified call');
assert(interactionPresentation({ ...unifiedAction, arguments: '{"targetKind":"window","action":"capture_window"}', capabilities: { ...completed.capabilities!, readOnly: true } })?.tone === 'neutral', 'a host-confirmed observation is complete without claiming the user goal matched');
console.log('tool visual evidence contracts passed');
