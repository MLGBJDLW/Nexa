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
  completed: ReadonlyMap<string, NonNullable<GenerationMetrics>>;
  totals: Readonly<Partial<Record<'request' | 'turn', NonNullable<GenerationMetrics>>>>;
  metrics: GenerationMetrics;
  blocked: GenerationMetrics;
}

function sameMeasurement(a: GenerationMetrics, b: GenerationMetrics): boolean {
  return a?.seriesId === b?.seriesId && a?.outputTokens === b?.outputTokens
    && a?.requestDurationMs === b?.requestDurationMs && !!a?.externalTurn === !!b?.externalTurn;
}

function seriesKey(metrics: NonNullable<GenerationMetrics>): string {
  return metrics.seriesId ?? `legacy:${!!metrics.externalTurn}:${metrics.outputTokens}:${metrics.requestDurationMs}`;
}

/** Cumulative snapshots replace their series contribution. The active series
 * updates in constant time; only crossing turns or correcting an older turn
 * copies the index. Replayed durable snapshots leave the displayed series alone. */
export function retainGenerationSample(
  previous: GenerationSample | null,
  scope: string,
  observed: GenerationMetrics,
): GenerationSample {
  if (previous?.scope === scope && sameMeasurement(previous.observed, observed)) return previous;
  const state: GenerationSample = previous?.scope === scope
    ? { ...previous, observed }
    : { scope, observed, latest: undefined, completed: new Map(), totals: {}, metrics: undefined, blocked: previous?.latest ?? previous?.blocked };
  if (!observed || modelTokensPerSecond(observed) == null) return state;
  const key = seriesKey(observed);
  if (state.blocked && seriesKey(state.blocked) === key) return state;
  const { latest, completed } = state;
  const archived = completed.get(key);
  const prior = latest && seriesKey(latest) === key ? latest : archived;
  if (prior && observed.requestDurationMs <= prior.requestDurationMs) return state;

  const kind = observed.externalTurn ? 'turn' : 'request';
  const total = state.totals[kind];
  state.totals = {
    ...state.totals,
    [kind]: {
      outputTokens: (total?.outputTokens ?? 0) + observed.outputTokens - (prior?.outputTokens ?? 0),
      requestDurationMs: (total?.requestDurationMs ?? 0) + observed.requestDurationMs - (prior?.requestDurationMs ?? 0),
      externalTurn: observed.externalTurn,
    },
  };
  if (archived) {
    state.completed = new Map(completed).set(key, observed);
  } else {
    if (latest && !prior) state.completed = new Map(completed).set(seriesKey(latest), latest);
    state.latest = observed;
  }
  state.metrics = state.totals[state.latest?.externalTurn ? 'turn' : 'request'];
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
