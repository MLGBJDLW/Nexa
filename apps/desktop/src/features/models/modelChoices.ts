export interface TurnModelSelection {
  model: string;
  reasoningEnabled: boolean | null;
  reasoningEffort: string | null;
  thinkingBudget: number | null;
}
export interface ModelChoice {
  id: string;
  name: string;
  vision: boolean | null;
  reasoning: {
    mode?: string | null;
    effortLevels: string[];
    defaultEffort?: string | null;
    thinkingBudget?: { enabled?: boolean; minTokens?: number; maxTokens?: number; defaultTokens?: number } | null;
  } | null;
}
export interface ModelChoices {
  connectionId: string;
  models: ModelChoice[];
  discoverySucceeded: boolean;
  error: string | null;
}
export type ModelChoicesLoader = (connectionId: string, refresh?: boolean) => Promise<ModelChoices>;

/** Each transport owns its account/endpoint-scoped cache. Never share across paired hosts. */
export function cachedModelChoices(load: (id: string) => Promise<ModelChoices>): ModelChoicesLoader {
  const entries = new Map<string, { at: number; value?: ModelChoices; pending?: Promise<ModelChoices> }>();
  return (id, refresh = false) => {
    const current = entries.get(id);
    if (current?.pending) return current.pending;
    if (!refresh && current?.value && Date.now() - current.at < 120_000) return Promise.resolve(current.value);
    const entry = { at: Date.now(), value: current?.value, pending: undefined as Promise<ModelChoices> | undefined };
    entry.pending = load(id).then(value => {
      if (!value || !Array.isArray(value.models) || value.connectionId !== id) throw new Error('Invalid model catalog');
      entry.value = value;
      entry.at = Date.now();
      return value;
    }).finally(() => { entry.pending = undefined; });
    entries.set(id, entry);
    return entry.pending;
  };
}
