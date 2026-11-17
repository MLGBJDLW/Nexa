/** Fold common model-ID separators while preserving letters, digits and symbols such as '+'. */
export function compactModelSearch(value: string): string {
  return value.normalize('NFKC').toLowerCase().replace(/[\s_.:/\-]+/g, '');
}
