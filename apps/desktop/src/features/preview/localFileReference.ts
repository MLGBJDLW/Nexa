export function localFileReference(value: string): string {
  const path = value.startsWith('file:') ? value.slice(5).replace(/^\/{3}(?=[A-Za-z]:)/, '') : value;
  try { return decodeURIComponent(path); } catch { return path; }
}

/** Automatic image loads must never interpret a network/device path as local. */
export function isInlineFileReference(path: string): boolean {
  return !path.replace(/\\/g, '/').startsWith('//')
    && (!/^[a-z][a-z0-9+.-]*:/i.test(path) || /^[a-z]:[\\/]/i.test(path));
}
