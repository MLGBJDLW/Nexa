import type { TraceEvent, TraceReplyEvent, TraceThinkingEvent } from './protocol';

type TextTraceEvent = TraceReplyEvent | TraceThinkingEvent;
type TextTraceKind = TextTraceEvent['kind'];
const encoder = new TextEncoder();
const decoder = new TextDecoder();

export function blockTraceText(events: TraceEvent[], kind: TextTraceKind, blockId: string): string {
  return events.filter((event): event is TextTraceEvent => event.kind === kind && event.blockId === blockId)
    .map(event => event.text).join('');
}

/** One wire block may span several chronological presentation segments. */
export function appendBlockTraceDelta(
  events: TraceEvent[], kind: TextTraceKind, blockId: string,
  offset: number, delta: string, newId: () => string,
): TraceEvent[] {
  let index = -1;
  for (let candidate = events.length - 1; candidate >= 0; candidate--) {
    const event = events[candidate];
    if (event.kind === kind && event.blockId === blockId) { index = candidate; break; }
  }
  const previous = index >= 0 ? events[index] as TextTraceEvent : undefined;
  if (previous && offset < (previous.nextOffset ?? 0)) return events;
  const nextOffset = offset + encoder.encode(delta).length;
  const crossedBoundary = events.slice(index + 1).some(event => event.kind === 'tool'
    || (event.kind === 'status' && event.displayKind === 'steering'));
  if (previous && !crossedBoundary) {
    const next = [...events];
    next[index] = { ...previous, text: previous.text + delta, nextOffset };
    return next;
  }
  return [...events, { id: newId(), kind, blockId, startOffset: offset, nextOffset, text: delta }];
}

function characterBoundary(bytes: Uint8Array, offset: number): number {
  let boundary = Math.max(0, Math.min(bytes.length, offset));
  while (boundary > 0 && boundary < bytes.length && (bytes[boundary] & 0xc0) === 0x80) boundary--;
  return boundary;
}

/** Correct a complete block once while retaining its existing tool boundaries.
 * Old byte cutoffs can land inside a corrected Unicode character, so clamp
 * them to codepoint boundaries before projecting each non-overlapping slice. */
export function replaceBlockTraceText(
  events: TraceEvent[], kind: TextTraceKind, blockId: string, text: string,
): TraceEvent[] {
  const indices = events.flatMap((event, index) => event.kind === kind && event.blockId === blockId ? [index] : []);
  if (indices.length === 0) return events;
  const bytes = encoder.encode(text);
  const next = [...events];
  let start = 0;
  indices.forEach((index, segment) => {
    const previous = events[index] as TextTraceEvent;
    const end = segment === indices.length - 1 ? bytes.length
      : Math.max(start, characterBoundary(bytes, previous.nextOffset ?? encoder.encode(previous.text).length));
    next[index] = { ...previous, startOffset: start, nextOffset: end, text: decoder.decode(bytes.subarray(start, end)) };
    start = end;
  });
  return next;
}
