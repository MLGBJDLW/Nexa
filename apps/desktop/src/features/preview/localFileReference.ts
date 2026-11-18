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

/** Direct text reads bind the line selection to the bytes read by the tool. */
export function fileLinkTarget(href: string): { path: string; position?: PreviewLocation } {
  // A file URL has an empty authority before its absolute POSIX or drive path.
  const target = href.startsWith('file:///') ? href.slice(7).replace(/^\/(?=[A-Za-z]:)/, '') : href;
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
