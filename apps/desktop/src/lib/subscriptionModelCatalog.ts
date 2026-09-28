import { invoke } from '@tauri-apps/api/core';
import type { CopilotModelSummary } from './api';

export interface SubscriptionCatalogState {
  provider: string;
  models: CopilotModelSummary[] | null;
  loading: boolean;
  error: string | null;
  updatedAt: number;
}

const TTL_MS = 5 * 60_000;
const RETRY_DELAY_MS = 30_000;
const REQUEST_TIMEOUT_MS = 30_000;
const listeners = new Set<() => void>();
let snapshots: Record<string, SubscriptionCatalogState> = {};
const pending = new Map<string, Promise<CopilotModelSummary[]>>();
const generations = new Map<string, number>();
const accountIdentities = new Map<string, string | null>();

export function reconcileSubscriptionAccount(provider: string, identity: string | null): void {
  const previous = accountIdentities.get(provider);
  accountIdentities.set(provider, identity);
  if (previous !== undefined && previous !== identity) invalidateSubscriptionModels(provider);
}

export const getSubscriptionCatalogs = () => snapshots;
export function subscribeSubscriptionCatalogs(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

function publish(provider: string, state: SubscriptionCatalogState): void {
  snapshots = { ...snapshots, [provider]: state };
  listeners.forEach(listener => listener());
}

/** Native subscription credentials are owned by one account per runtime.
 * Keep this cache in memory and invalidate it across account transitions.
 * A late result from the previous account must never repopulate the cache.
 */
export function invalidateSubscriptionModels(provider: string): void {
  generations.set(provider, (generations.get(provider) ?? 0) + 1);
  pending.delete(provider);
  const next = { ...snapshots };
  delete next[provider];
  snapshots = next;
  listeners.forEach(listener => listener());
}

export function runtimeCatalogKey(provider: string, agentConfigId?: string): string {
  return agentConfigId ? JSON.stringify([provider, agentConfigId]) : provider;
}

export function loadSubscriptionModels(provider: string, force = false, agentConfigId?: string): Promise<CopilotModelSummary[]> {
  const key = runtimeCatalogKey(provider, agentConfigId);
  const running = pending.get(key);
  if (running) return running;
  const cached = snapshots[key];
  const age = Date.now() - (cached?.updatedAt ?? 0);
  if (!force && cached && age < (cached.error ? RETRY_DELAY_MS : TTL_MS)) {
    return cached.error ? Promise.reject(new Error(cached.error)) : Promise.resolve(cached.models ?? []);
  }
  const generation = generations.get(key) ?? 0;
  let timer: ReturnType<typeof setTimeout>;
  const request = Promise.race([
    invoke<CopilotModelSummary[]>('list_subscription_models_cmd', { provider, ...(agentConfigId ? { agentConfigId } : {}) }),
    new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error('Model catalog request timed out. Please retry.')), REQUEST_TIMEOUT_MS);
    }),
  ]).then(models => {
    if ((generations.get(key) ?? 0) !== generation) throw new Error('Subscription account changed. Refresh models.');
    publish(key, { provider, models, loading: false, error: null, updatedAt: Date.now() });
    return models;
  }).catch(error => {
    if ((generations.get(key) ?? 0) === generation) {
      publish(key, { provider, models: cached?.models ?? null, loading: false, error: String(error), updatedAt: Date.now() });
    }
    throw error;
  }).finally(() => {
    clearTimeout(timer);
    if (pending.get(key) === request) pending.delete(key);
  });
  pending.set(key, request);
  publish(key, { provider, models: cached?.models ?? null, loading: true, error: null, updatedAt: cached?.updatedAt ?? 0 });
  return request;
}
