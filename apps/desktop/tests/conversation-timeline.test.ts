import { ConversationTimeline } from '../src/lib/conversationTimeline';
import type { ConversationMessage, ConversationTimelineDetails, ConversationTimelinePage } from '../src/types/conversation';

function assert(condition: unknown, message: string): asserts condition { if (!condition) throw new Error(message); }
const cursor = (n: number) => ({ sortOrder: n * 10, messageId: `u${n}` });
const message = (n: number, role: ConversationMessage['role'] = 'user'): ConversationMessage => ({ id: role === 'user' ? `u${n}` : `a${n}`, conversationId: 'chat', role, content: `${role}${n}`, toolCallId: null, toolCalls: [], artifacts: null, tokenCount: 0, createdAt: 'same-second', sortOrder: n*10+(role === 'user' ? 0 : 1), thinking: null });
const page = (numbers: number[], before: number | null = null): ConversationTimelinePage => ({
  conversation: { id: 'chat', title: '', provider: '', model: '', systemPrompt: '', createdAt: '', updatedAt: '' },
  messages: numbers.flatMap(n => [message(n),message(n,'assistant')]),
  turns: numbers.map(n => ({ id: `t${n}`, conversationId: 'chat', userMessageId: `u${n}`, assistantMessageId: `a${n}`, status: 'completed', createdAt: '', updatedAt: '1' })),
  entries: numbers.map(n => ({ anchor: cursor(n), turnId: `t${n}`, hasDetails: true, detailRevision: '1' })),
  taskRuns: [], range: numbers.length ? { from: cursor(numbers[0]), before: before == null ? null : cursor(before) } : null,
  oldestCursor: numbers.length ? cursor(numbers[0]) : null, newestCursor: numbers.length ? cursor(numbers[numbers.length-1]) : null,
  hasMoreBefore: numbers[0] > 1, hasMoreAfter: before != null,
});
const deferred = <T,>() => { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; };
const details = (n: number): ConversationTimelineDetails => ({ anchorId: `u${n}`, messages: [message(n), { ...message(n,'tool'), id: `tool${n}`, content: 'lazy tool output' }, message(n,'assistant')], turns: [{ ...page([n]).turns[0], trace: { items: ['lazy trace'] } }], range: { from: cursor(n), before: cursor(n+1) }, detailRevision: '1' });

async function main() {
  const requests: unknown[] = [];
  let nextPage = page([9,10]);
  let detailReads = 0;
  let nextDetails: Promise<ConversationTimelineDetails> = Promise.resolve(details(9));
  const timeline = new ConversationTimeline({ page: async (_id, request) => { requests.push(request); return nextPage; }, details: async () => { detailReads++; return nextDetails; }, mergeLocalMessages: (_previous,next) => next, protectedIds: () => [] });
  await timeline.openTail('chat');
  assert(detailReads === 0 && timeline.get('chat').messages.length === 4, 'opening history must not read hidden details');
  const oldMessage = timeline.get('chat').messages[0];
  nextPage = page([7,8],9);
  await timeline.loadBefore('chat');
  assert(timeline.get('chat').messages.find(item => item.id === 'u9') === oldMessage, 'prepend must preserve existing row identity');
  assert(timeline.get('chat').newestCursor?.messageId === 'u10', 'prepend must not move tail cursor');
  await timeline.loadDetails('chat','u9');
  assert(Number(detailReads) === 1 && timeline.get('chat').messages.some(item => item.id === 'tool9'), 'explicit expansion reads detail once');
  await timeline.loadDetails('chat','u9');
  assert(Number(detailReads) === 1, 'loaded details must not be fetched twice');
  nextPage = page([10,11]);
  await timeline.refreshTail('chat');
  assert(JSON.stringify(requests[requests.length-1]) === JSON.stringify({ after: cursor(10) }), 'completion must fetch only the inclusive suffix');
  assert(timeline.get('chat').messages.find(item => item.id === 'u9') === oldMessage || timeline.get('chat').messages.some(item => item.id === 'tool9'), 'suffix must retain older loaded detail');
  assert(timeline.get('chat').turns.map(turn => turn.id).join(',') === 't7,t8,t9,t10,t11', 'turn order follows durable anchor order');
  const stale = deferred<ConversationTimelineDetails>();
  nextDetails = stale.promise;
  const pending = timeline.loadDetails('chat','u10');
  timeline.invalidate('chat');
  nextPage = page([10]);
  nextPage.entries[0].detailRevision = '2';
  await timeline.openTail('chat');
  stale.resolve(details(10)); await pending;
  assert(!timeline.get('chat').messages.some(item => item.id === 'tool10'), 'late detail cannot overwrite retry history');
  assert(timeline.get('chat').entries[0].detailRevision === '2', 'retry revision survives stale detail');

  const pendingPage = deferred<ConversationTimelinePage>();
  const clearing = new ConversationTimeline({ page: () => pendingPage.promise, details: async () => details(1), mergeLocalMessages: (_p,n) => n, protectedIds: () => [] });
  const opening = clearing.openTail('uncached'); clearing.clearAll(); pendingPage.resolve(page([1])); await opening;
  assert(clearing.get('uncached').messages.length === 0, 'delete-all fences an uncached in-flight first page');
  nextPage = page([3,4],5); await timeline.openTail('chat','a3');
  assert(JSON.stringify(requests[requests.length-1]) === JSON.stringify({ anchorMessageId: 'a3' }) && timeline.get('chat').hasMoreAfter, 'deep-link opens its bounded anchor page');

  const staleOlder = deferred<ConversationTimelinePage>();
  const freshOlder = deferred<ConversationTimelinePage>();
  const staleDetails = deferred<ConversationTimelineDetails>();
  const freshDetails = deferred<ConversationTimelineDetails>();
  let olderReads = 0;
  let detailAttempts = 0;
  const cancelled = new ConversationTimeline({
    page: async (_id, request) => request?.before
      ? (++olderReads === 1 ? staleOlder.promise : freshOlder.promise)
      : page([9,10]),
    details: async () => ++detailAttempts === 1 ? staleDetails.promise : freshDetails.promise,
    mergeLocalMessages: (_previous, next) => next,
    protectedIds: () => [],
  });
  await cancelled.openTail('chat');
  const abandonedOlder = cancelled.loadBefore('chat');
  const abandonedDetails = cancelled.loadDetails('chat', 'u9');
  assert(cancelled.get('chat').loadingOlder && cancelled.get('chat').entries[0].detailsLoading, 'both requests own visible loading indicators');
  // Sending a new turn fences display reads before adding optimistic content.
  cancelled.fence('chat');
  assert(!cancelled.get('chat').loadingOlder && !cancelled.get('chat').entries[0].detailsLoading, 'fencing releases abandoned loading indicators');
  const retriedOlder = cancelled.loadBefore('chat');
  const retriedDetails = cancelled.loadDetails('chat', 'u9');
  assert(olderReads === 2 && detailAttempts === 2, 'both readers can retry immediately after cancellation');
  staleOlder.resolve(page([1,2],3));
  staleDetails.resolve(details(9));
  await Promise.all([abandonedOlder, abandonedDetails]);
  assert(cancelled.get('chat').loadingOlder && cancelled.get('chat').entries[0].detailsLoading, 'old completions cannot clear newer loading indicators');
  assert(!cancelled.get('chat').messages.some(item => item.id === 'u1' || item.id === 'tool9'), 'cancelled reads cannot restore stale pages or details');
  freshOlder.resolve(page([7,8],9));
  freshDetails.resolve(details(9));
  await Promise.all([retriedOlder, retriedDetails]);
  assert(!cancelled.get('chat').loadingOlder && cancelled.get('chat').entries.find(entry => entry.anchor.messageId === 'u9')?.detailsLoaded, 'new requests finish after the earlier generation was cancelled');
  await cancelled.openTail('chat');
  assert(!cancelled.get('chat').loadingOlder, 'reopening a tail must not inherit an abandoned older-page spinner');
  console.log('conversation timeline contracts passed');
}
void main().catch(error => { console.error(error); throw error; });
