import { useEffect, useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { applyExternalAgentPresets, type ProviderPreset } from './providerPresets';

export interface AgentRegistryEntry {
  preset: ProviderPreset;
  availableVersion: string | null;
  updateAvailable: boolean;
  fingerprint: string | null;
}
interface RegistrySnapshot { source: string; checkedAt: number | null; entries: AgentRegistryEntry[] }
interface RegistryState { snapshot: RegistrySnapshot | null; checking: boolean; error: string | null }
let state: RegistryState = { snapshot: null, checking: false, error: null };
let initial: Promise<void> | null = null;
let refresh: Promise<void> | null = null;
let requestGeneration = 0;
const listeners = new Set<() => void>();
function publish(next: RegistryState) { state = next; listeners.forEach(listener => listener()); }
function accept(snapshot: RegistrySnapshot | null) {
  if (!snapshot || !Array.isArray(snapshot.entries)) return;
  const entries = snapshot.entries.map(entry => ({ ...entry, preset: {
    ...entry.preset, runtime: 'acp' as const, baseUrl: '', models: [], requiresApiKey: false,
    icon: entry.preset.icon || '/provider-icons/agents/custom.svg',
  } }));
  applyExternalAgentPresets(entries.map(entry => entry.preset));
  publish({ ...state, snapshot: { ...snapshot, entries }, error: null });
}
function load() {
  initial ??= invoke<RegistrySnapshot>('get_external_agent_registry_cmd').then(snapshot => {
    if (!state.snapshot) accept(snapshot);
  }).catch(() => {}).then(() => {});
  return initial;
}
export async function checkExternalAgentUpdates(force = true): Promise<void> {
  if (refresh) return refresh;
  const generation = ++requestGeneration;
  publish({ ...state, checking: true, error: null });
  refresh = invoke<RegistrySnapshot>('check_external_agent_updates_cmd', { force })
    .then(snapshot => { if (generation === requestGeneration) accept(snapshot); })
    .catch(error => { if (force && generation === requestGeneration) publish({ ...state, error: String(error) }); })
    .finally(() => { refresh = null; publish({ ...state, checking: false }); });
  return refresh;
}
export async function updateExternalAgent(provider: string, fingerprint: string, requestId: string): Promise<void> {
  // A slower directory response must not restore a selection made before the
  // successful activation. The host remains the authority for every preset.
  const generation = ++requestGeneration;
  const snapshot = await invoke<RegistrySnapshot>('update_external_agent_cmd', { provider, fingerprint, requestId });
  if (generation === requestGeneration) accept(snapshot);
}
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
const getSnapshot = () => state;
export function useExternalAgentRegistry() {
  useEffect(() => { void load(); }, []);
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
