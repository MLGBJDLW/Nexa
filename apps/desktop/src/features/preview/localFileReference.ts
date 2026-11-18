import type { EvidenceLocator } from '../../types/evidence';
import type { PreviewLocation } from './filePreviewContext';

export function localFileReference(value: string): string {
  const path = value.startsWith('file:') ? value.slice(5).replace(/^\/{3}(?=[A-Za-z]:)/, '') : value;
  try { return decodeURIComponent(path); } catch { return path; }
}

/** Automatic image loads must never interpret a network/device path as local. */
export function isInlineFileReference(path: string): boolean {
  return !path.replace(/\\/g, '/').startsWith('//')
    && (!/^[a-z][a-z0-9+.-]*:/i.test(path) || /^[a-z]:[\\/]/i.test(path));
}

function isSourcePosition(value: unknown): value is EvidenceLocator {
  if (!value || typeof value !== 'object') return false;
  const loc = value as Record<string, unknown>;
  const positive = (n: unknown): n is number => typeof n === 'number' && Number.isSafeInteger(n) && n > 0;
  switch (loc.kind) {
    case 'pdf':
      return positive(loc.page) && (loc.extractedLineStart == null && loc.extractedLineEnd == null
        || positive(loc.extractedLineStart) && positive(loc.extractedLineEnd) && loc.extractedLineStart <= loc.extractedLineEnd);
    case 'slide': return positive(loc.slide);
    case 'sheet': return typeof loc.sheet === 'string' && typeof loc.range === 'string' && /^[A-Z]+[1-9]\d*(:[A-Z]+[1-9]\d*)?$/.test(loc.range)
      && (loc.contextRange == null || typeof loc.contextRange === 'string' && /^[A-Z]+[1-9]\d*(:[A-Z]+[1-9]\d*)?$/.test(loc.contextRange));
    case 'document':
      return typeof loc.part === 'string' && positive(loc.paragraph) && [loc.table, loc.row, loc.column, loc.contextRow].every(n => n == null || positive(n));
    default: return false;
  }
}

/** Direct file links bind a source position to the bytes returned by the tool. */
export function fileLinkTarget(href: string): { path: string; position?: PreviewLocation } {
  const target = href.startsWith('file:///') ? href.slice(7).replace(/^\/(?=[A-Za-z]:)/, '') : href;
  const fragment = href.startsWith('file://') ? target.indexOf('#loc=') : -1;
  if (fragment >= 0) {
    const path = localFileReference(target.slice(0, fragment));
    const params = new URLSearchParams(target.slice(fragment + 1));
    const expectedHash = params.get('rev') ?? '';
    try {
      const locator: unknown = JSON.parse(params.get('loc') ?? '');
      if (/^[0-9a-f]{64}$/.test(expectedHash) && isSourcePosition(locator)) return { path, position: { expectedHash, locator } };
    } catch { /* Open the file when a saved or hand-written position is invalid. */ }
    return { path };
  }
  const match = href.startsWith('file://') && /#L([1-9]\d*)-L([1-9]\d*)@([0-9a-f]{64})$/.exec(target);
  if (match) {
    const lineStart = Number(match[1]), lineEnd = Number(match[2]);
    const path = localFileReference(target.slice(0, match.index));
    if (Number.isSafeInteger(lineEnd) && lineStart <= lineEnd) {
      return { path, position: { expectedHash: match[3], locator: { kind: 'text', lineStart, lineEnd } } };
    }
    return { path };
  }
  return { path: localFileReference(target) };
}
