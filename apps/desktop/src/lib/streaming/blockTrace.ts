import type { TraceEvent, TraceReplyEvent, TraceThinkingEvent } from './protocol';

export type TextTraceEvent = TraceReplyEvent | TraceThinkingEvent;
type TextTraceKind = TextTraceEvent['kind'];
const encoder = new TextEncoder();
const decoder = new TextDecoder();

export function utf8ByteLength(text: string): number {
  return encoder.encode(text).length;
}

/** Validate replay overlap before taking the missing suffix. `textStart` lets
 * bounded clients retain only the tail needed for a compacted durable row. */
export function recoveredDeltaSuffix(text: string, offset: number, consumedThrough: number, delta: string, textStart = 0): string | null {
  if (!Number.isSafeInteger(offset) || offset < textStart || offset > consumedThrough) return null;
  const incoming = encoder.encode(delta);
  const overlap = Math.min(consumedThrough - offset, incoming.length);
  const existing = encoder.encode(text);
  if (offset - textStart + overlap > existing.length) return null;
  for (let index = 0; index < overlap; index++) {
    if (incoming[index] !== existing[offset - textStart + index]) return null;
  }
  try { return new TextDecoder('utf-8', { fatal: true }).decode(incoming.subarray(overlap)); }
  catch { return null; }
}

export function blockTraceText(events: TraceEvent[], kind: TextTraceKind, blockId: string): string {
  let text = '';
  for (const event of events) if (event.kind === kind && event.blockId === blockId) text += event.text;
  return text;
}

/** One wire block may span several chronological presentation segments. */
export function blockTraceDelta(
  events: TraceEvent[], kind: TextTraceKind, blockId: string,
  offset: number, nextOffset: number, delta: string, newId: () => string,
): { index: number; event: TextTraceEvent } | null {
  let index = -1;
  let crossedBoundary = false;
  for (let candidate = events.length - 1; candidate >= 0; candidate--) {
    const event = events[candidate];
    if (event.kind === kind && event.blockId === blockId) { index = candidate; break; }
    if (event.kind === 'tool' || (event.kind === 'status' && event.displayKind === 'steering')) crossedBoundary = true;
  }
  const previous = index >= 0 ? events[index] as TextTraceEvent : undefined;
  if (previous && offset < (previous.nextOffset ?? 0)) return null;
  if (previous && !crossedBoundary) {
    return { index, event: { ...previous, text: previous.text + delta, nextOffset } };
  }
  return { index: events.length, event: { id: newId(), kind, blockId, startOffset: offset, nextOffset, text: delta } };
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
  const indices: number[] = [];
  for (let index = 0; index < events.length; index++) {
    const event = events[index];
    if (event.kind === kind && event.blockId === blockId) indices.push(index);
  }
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
