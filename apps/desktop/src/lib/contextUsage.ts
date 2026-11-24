import type { ContextUsageBreakdown } from '../types/conversation';

export function modelTokensPerSecond(metrics: ContextUsageBreakdown['generation']): number | null {
  if (!metrics || !Number.isSafeInteger(metrics.outputTokens) || metrics.outputTokens < 0
    || !Number.isSafeInteger(metrics.requestDurationMs) || metrics.requestDurationMs <= 0) return null;
  return metrics.outputTokens * 1000 / metrics.requestDurationMs;
}

type GenerationMetrics = ContextUsageBreakdown['generation'];
export interface GenerationSample {
  scope: string;
  observed: GenerationMetrics;
  latest: GenerationMetrics;
  metrics: GenerationMetrics;
  blocked: GenerationMetrics;
}

function sameMeasurement(a: GenerationMetrics, b: GenerationMetrics): boolean {
  return a?.seriesId === b?.seriesId && a?.outputTokens === b?.outputTokens
    && a?.requestDurationMs === b?.requestDurationMs && !!a?.externalTurn === !!b?.externalTurn;
}

/** Accumulate measured tokens and time. New snapshots of an active series
 * replace its contribution, including when a live event is replayed from disk. */
export function retainGenerationSample(
  previous: GenerationSample | null,
  scope: string,
  observed: GenerationMetrics,
): GenerationSample {
  if (previous?.scope === scope && previous.observed === observed) return previous;
  const state: GenerationSample = previous?.scope === scope
    ? { ...previous, observed }
    : { scope, observed, latest: undefined, metrics: undefined, blocked: previous?.latest ?? previous?.blocked };
  if (!observed || modelTokensPerSecond(observed) == null) return state;
  const { latest, blocked } = state;
  if (blocked && (blocked.seriesId ? blocked.seriesId === observed.seriesId : sameMeasurement(blocked, observed))) return state;
  if (sameMeasurement(latest, observed)) return state;
  const replacing = !!observed.seriesId && latest?.seriesId === observed.seriesId;
  if (replacing && observed.requestDurationMs < latest.requestDurationMs) return state;
  const previousTotal = !!state.metrics?.externalTurn === !!observed.externalTurn ? state.metrics : undefined;
  state.metrics = {
    ...observed,
    outputTokens: (previousTotal?.outputTokens ?? 0) + observed.outputTokens - (replacing ? latest.outputTokens : 0),
    requestDurationMs: (previousTotal?.requestDurationMs ?? 0) + observed.requestDurationMs - (replacing ? latest.requestDurationMs : 0),
  };
  state.latest = observed;
  return state;
}

interface UsageInput {
  promptTokens: number;
  contextWindow: number;
  contextBreakdown?: ContextUsageBreakdown;
}

/** Capacity and trigger are a projection of the runtime, never UI policy. */
export function resolveContextUsage(usage: UsageInput | null | undefined) {
  const candidate = usage?.contextBreakdown?.budget;
  const budget = candidate
    && Object.values(candidate).every(value => Number.isSafeInteger(value) && value >= 0)
    && candidate.capacityTokens > 0
    && candidate.inputBudget + candidate.responseReserve + candidate.safetyReserve === candidate.capacityTokens
    && candidate.compactPercent >= 60 && candidate.compactPercent <= 95
    && candidate.compactThreshold === Math.floor(candidate.inputBudget * candidate.compactPercent / 100)
    ? candidate : undefined;
  const reported = usage?.contextBreakdown?.contextWindow;
  const runtimeCapacity = typeof reported === 'number' && Number.isSafeInteger(reported) && reported > 0 ? reported : undefined;
  const capacity = budget?.capacityTokens ?? runtimeCapacity ?? usage?.contextWindow ?? 0;
  const inputBudget = budget?.inputBudget ?? capacity;
  const used = Math.max(0, usage?.promptTokens ?? 0);
  const percent = inputBudget > 0 ? Math.min(100, used / inputBudget * 100) : used > 0 ? 100 : 0;
  return {
    budget, capacity, inputBudget, used, percent,
    compactPercent: budget?.compactPercent ?? null,
    warningPercent: Math.min(80, budget?.compactPercent ?? 80),
    untilCompact: budget ? Math.max(0, budget.compactThreshold - used) : null,
  };
}
