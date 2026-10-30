/** Project a small activity label without parsing or copying the full trace. */
export function thinkingSummary(text: string): string {
  let tail = text.slice(-1600);
  if (text.length > tail.length) {
    const boundary = tail.indexOf('\n');
    tail = boundary >= 0 ? tail.slice(boundary + 1) : tail;
  }
  const paragraphs = tail.trim().split(/\n\s*\n/);
  const paragraph = paragraphs[paragraphs.length - 1] ?? '';
  const line = paragraph.split('\n').find(line => line.trim() && !/^\s*```/.test(line)) ?? '';
  return compactActivityLabel(line);
}

export function compactActivityLabel(text: string): string {
  const plain = text.slice(0, 1600)
    .replace(/^[\uDC00-\uDFFF]/, '')
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/^\s*(?:#{1,6}\s+|[-*>]\s+|\d+[.)]\s+)/, '')
    .replace(/[*`_]/g, '')
    .replace(/\s+/g, ' ')
    .trim();
  const chars = Array.from(plain);
  return chars.length > 100 ? `${chars.slice(0, 99).join('').trimEnd()}…` : plain;
}
