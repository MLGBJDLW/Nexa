export function localFileReference(value: string): string {
  const path = value.startsWith('file:') ? value.slice(5).replace(/^\/{3}(?=[A-Za-z]:)/, '') : value;
  try { return decodeURIComponent(path); } catch { return path; }
}
