/** Preserve exact fenced/span source across prose-only transformations. An
 * unclosed streaming fence protects the remaining source until it closes. */
function segments(content: string): { text: string; code: boolean }[] {
  const result: { text: string; code: boolean }[] = [];
  let fence: { marker: string; length: number } | null = null;
  let pending = ''; let code = false;
  const flush = () => { if (pending) result.push({ text: pending, code }); pending = ''; };
  for (const line of content.match(/[^\n]*\n|[^\n]+$/g) ?? []) {
    const match = line.match(/^(?:(?:\s*>[ \t]?)|(?:\s*(?:[-+*]|\d{1,9}[.)])[ \t]+))*[ \t]*(`{3,}|~{3,})([^\r\n]*)/);
    if (!fence && match && !(match[1][0] === '`' && match[2].includes('`'))) {
      flush(); code = true; fence = { marker: match[1][0], length: match[1].length }; pending = line;
    } else if (fence && match && match[1][0] === fence.marker && match[1].length >= fence.length && !match[2].trim()) {
      pending += line; flush(); fence = null; code = false;
    } else pending += line;
  }
  flush();
  return result.flatMap(part => {
    if (part.code) return [part];
    const spans: typeof result = []; let cursor = 0;
    const ticks = /`+/g; let match: RegExpExecArray | null;
    while ((match = ticks.exec(part.text))) {
      if (match.index > 0 && part.text[match.index - 1] === '\\') continue;
      const start = match.index; const width = match[0].length;
      let closing: RegExpExecArray | null; let end = part.text.length;
      while ((closing = ticks.exec(part.text))) {
        if (closing[0].length === width) { end = ticks.lastIndex; break; }
      }
      if (start > cursor) spans.push({ text: part.text.slice(cursor, start), code: false });
      spans.push({ text: part.text.slice(start, end), code: true }); cursor = end;
      if (!closing) break;
    }
    if (cursor < part.text.length) spans.push({ text: part.text.slice(cursor), code: false });
    return spans;
  });
}

export function transformMarkdownProse(content: string, transform: (prose: string) => string): string {
  let prefix = '\u0000nexa-code-'; while (content.includes(prefix)) prefix += 'x';
  const saved: string[] = [];
  const masked = segments(content).map(part => {
    if (!part.code) return part.text;
    saved.push(part.text); return `${prefix}${saved.length - 1}\u0000`;
  }).join('');
  return transform(masked).replace(new RegExp(prefix + '(\\d+)\\u0000', 'g'), (_, index) => saved[Number(index)]);
}

export function markdownProse(content: string): string {
  return segments(content).map(part => part.code ? '\n' : part.text).join('');
}
