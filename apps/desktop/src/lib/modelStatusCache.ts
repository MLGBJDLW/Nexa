/**
 * Module-level cache for model-probe IPC results.
 *
 * Rationale: probes like `check_ffmpeg` spawn subprocesses (100-500ms on
 * Windows cold-path). Settings re-mounts re-run all 4 probes, causing visible
 * lag. Cache first result for 30min; self-heals on download/delete via
 * `invalidate(kind)`.
 */

export type ProbeKind = 'embed' | 'ocr' | 'whisper' | 'ffmpeg' | 'office';

interface Entry<T> {
  value: T;
  expiresAt: number;
}

const TTL_MS = 30 * 60 * 1000;

const cache = new Map<string, Entry<unknown>>();
const inFlight = new Map<string, Promise<unknown>>();

function buildKey(kind: ProbeKind, key: string): string {
  return `${kind}:${key}`;
}

export async function getModelStatus<T>(
  kind: ProbeKind,
  key: string,
  fetcher: () => Promise<T>,
): Promise<T> {
  const cacheKey = buildKey(kind, key);
  const now = Date.now();
  const existing = cache.get(cacheKey) as Entry<T> | undefined;
  if (existing && existing.expiresAt > now) {
    return existing.value;
  }
  const pending = inFlight.get(cacheKey) as Promise<T> | undefined;
  if (pending) {
    return pending;
  }
  const promise = fetcher()
    .then((value) => {
      cache.set(cacheKey, { value, expiresAt: Date.now() + TTL_MS });
      return value;
    })
    .finally(() => {
      inFlight.delete(cacheKey);
    });
  inFlight.set(cacheKey, promise);
  return promise;
}

export function invalidate(kind: ProbeKind, key?: string): void {
  if (key !== undefined) {
    cache.delete(buildKey(kind, key));
    inFlight.delete(buildKey(kind, key));
    return;
  }
  const prefix = `${kind}:`;
  for (const k of cache.keys()) {
    if (k.startsWith(prefix)) cache.delete(k);
  }
  for (const k of inFlight.keys()) {
    if (k.startsWith(prefix)) inFlight.delete(k);
  }
}

export function invalidateAll(): void {
  cache.clear();
  inFlight.clear();
}
