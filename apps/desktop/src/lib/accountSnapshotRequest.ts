/** One bounded account read shared by Chat and Settings. Account credentials
 * remain in the native runtime; successful snapshots are not cached here. */
const pending = new Map<string, Promise<unknown>>();

export function accountSnapshotRequest<T>(key: string, load: () => Promise<T>, timeoutMs = 40_000): Promise<T> {
  const existing = pending.get(key);
  if (existing) return existing as Promise<T>;
  let timer: ReturnType<typeof setTimeout>;
  const request = Promise.race([
    Promise.resolve().then(load),
    new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error('Account usage request timed out. Please retry.')), timeoutMs);
    }),
  ]).finally(() => {
    clearTimeout(timer);
    if (pending.get(key) === request) pending.delete(key);
  });
  pending.set(key, request);
  return request;
}
