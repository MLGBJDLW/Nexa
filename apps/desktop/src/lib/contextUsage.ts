import type { ContextUsageBreakdown } from '../types/conversation';

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
  const capacity = budget?.capacityTokens ?? usage?.contextWindow ?? 0;
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
