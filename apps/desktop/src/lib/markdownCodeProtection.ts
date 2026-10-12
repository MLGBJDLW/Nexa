import { fromMarkdown } from 'mdast-util-from-markdown';

type MarkdownNode = {
  type: string;
  position?: { start: { offset?: number }; end: { offset?: number } };
  children?: readonly MarkdownNode[];
};

/** Use the renderer's CommonMark parser to protect exact source ranges, including
 * unfinished streaming fences and container-relative indentation. Unmatched
 * inline backticks remain prose, as they do in the rendered message. */
function segments(content: string): { text: string; code: boolean }[] {
  const ranges: { start: number; end: number }[] = [];
  const visit = (node: MarkdownNode) => {
    if (node.type === 'code' || node.type === 'inlineCode') {
      const start = node.position?.start.offset;
      const end = node.position?.end.offset;
      if (start !== undefined && end !== undefined) ranges.push({ start, end });
      return;
    }
    node.children?.forEach(visit);
  };
  visit(fromMarkdown(content));
  const result: { text: string; code: boolean }[] = [];
  let cursor = 0;
  for (const { start, end } of ranges) {
    if (start > cursor) result.push({ text: content.slice(cursor, start), code: false });
    result.push({ text: content.slice(start, end), code: true });
    cursor = end;
  }
  if (cursor < content.length) result.push({ text: content.slice(cursor), code: false });
  return result;
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
