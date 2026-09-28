import type { AgentConfig } from '../types/conversation';

/** V2 is authoritative, including null (automatic/unlimited) values. */
export function subagentDefaults(config?: AgentConfig | null) {
  const limits = config?.delegationLimitsV2;
  return {
    maxParallel: (limits ? limits.maxParallel : config?.subagentMaxParallel) ?? 4,
    maxCallsPerTurn: (limits ? limits.maxCallsPerTurn : config?.subagentMaxCallsPerTurn) ?? null,
    tokenBudget: (limits ? limits.totalActualTokensSoftLimit : config?.subagentTokenBudget) ?? null,
  };
}
