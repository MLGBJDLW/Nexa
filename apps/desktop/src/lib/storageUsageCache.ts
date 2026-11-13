import type { StorageService, StorageUsage } from './api';

interface Entry {
  value?: StorageUsage;
  expiresAt: number;
  generation: number;
  pending?: Promise<StorageUsage>;
}

const entries = new WeakMap<StorageService, Entry>();
const MAX_AGE_MS = 30_000;

function entryFor(service: StorageService): Entry {
  let entry = entries.get(service);
  if (!entry) {
    entry = { expiresAt: 0, generation: 0 };
    entries.set(service, entry);
  }
  return entry;
}

/** Keep the last measurement visible while a tab remounts or refreshes. */
export function storageUsageSnapshot(service: StorageService): StorageUsage | null {
  return entryFor(service).value ?? null;
}

/** A mutation invalidates even a scan that started before it committed. */
export function invalidateStorageUsage(service: StorageService): void {
  const entry = entryFor(service);
  entry.expiresAt = 0;
  entry.generation++;
}

export function readStorageUsage(service: StorageService): Promise<StorageUsage> {
  const entry = entryFor(service);
  if (entry.pending) return entry.pending;
  if (entry.value && entry.expiresAt > Date.now()) return Promise.resolve(entry.value);
  // Serialise refreshes as well as initial loads. A mutation during a scan
  // requests one follow-up measurement instead of launching parallel walkers.
  const request = Promise.resolve().then(async () => {
    for (;;) {
      const generation = entry.generation;
      const value = await service.usage();
      if (generation !== entry.generation) continue;
      entry.value = value;
      entry.expiresAt = Date.now() + MAX_AGE_MS;
      return value;
    }
  });
  entry.pending = request;
  const clear = () => { if (entry.pending === request) entry.pending = undefined; };
  void request.then(clear, clear);
  return request;
}
