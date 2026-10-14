/** One bounded account read shared by Chat and Settings. Account credentials
 * remain in the native runtime; successful snapshots are not cached here. */
const pending = new Map<string, Promise<unknown>>();
const invalidators = new Map<string, () => void>();

export function invalidateAccountSnapshot(key: string): void {
  invalidators.get(key)?.();
  invalidators.delete(key);
  pending.delete(key);
}

export function accountSnapshotRequest<T>(key: string, load: () => Promise<T>, timeoutMs = 40_000): Promise<T> {
  const existing = pending.get(key);
  if (existing) return existing as Promise<T>;
  let timer: ReturnType<typeof setTimeout>;
  const request = Promise.race([
    Promise.resolve().then(load),
    new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error('Account usage request timed out. Please retry.')), timeoutMs);
      invalidators.set(key, () => reject(new Error('Account changed. Please retry.')));
    }),
  ]).finally(() => {
    clearTimeout(timer);
    if (pending.get(key) === request) {
      pending.delete(key);
      invalidators.delete(key);
    }
  });
  pending.set(key, request);
  return request;
}
