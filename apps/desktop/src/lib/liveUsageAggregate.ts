import type { UsageSnapshot } from '../types/conversation';

const COUNTERS = ['promptTokens', 'completionTokens', 'totalTokens', 'thinkingTokens', 'cacheReadTokens', 'cacheMissTokens', 'cacheCreationTokens'] as const;
type RunUsage = { runId: string; usage: Record<string, unknown> };

function runsOf(snapshot: UsageSnapshot | null): RunUsage[] | null {
  const raw = snapshot?.providerRaw as { runs?: unknown } | null;
  if (!Array.isArray(raw?.runs)) return null;
  return raw.runs as RunUsage[];
}

/** Replace one run's cumulative contribution, including a run already present
 * in a reconnect snapshot. Never add two snapshots of the same run together. */
export function replaceRunUsage(base: UsageSnapshot | null, runId: string, run: UsageSnapshot): UsageSnapshot {
  const runs = runsOf(base) ?? [];
  const previous = runs.find(entry => entry.runId === runId)?.usage;
  const count = (value: unknown) => typeof value === 'number' && Number.isFinite(value) ? Math.max(0, value) : 0;
  const merged: UsageSnapshot = { ...run };
  for (const field of COUNTERS) {
    const before = field === 'totalTokens'
      ? Math.max(count(previous?.totalTokens), count(previous?.promptTokens) + count(previous?.completionTokens))
      : count(previous?.[field]);
    merged[field] = Math.max(0, count(base?.[field]) - before) + count(run[field]);
  }
  merged.source = base?.source === 'estimated' || run.source === 'estimated' ? 'estimated'
    : base?.source === 'normalized' || run.source === 'normalized' ? 'normalized' : 'provider';
  merged.providerRaw = { runs: [...runs.filter(entry => entry.runId !== runId), { runId, usage: run.providerRaw }] };
  return merged;
}
