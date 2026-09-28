import type { AgentConfig, SaveAgentConfigInput } from '../types/conversation';

/** External executors never receive an API credential or HTTP endpoint. */
export function runtimeAgentConfig(provider: string, name: string, model: string, config?: AgentConfig): SaveAgentConfigInput {
  const retained = config?.provider === provider && config.model === model;
  return {
    id: config?.id ?? null, name: name.trim(), provider, apiKey: '', baseUrl: null, model,
    modelId: model, providerEndpointId: null, temperature: null, maxTokens: null, contextWindow: null,
    isDefault: config?.isDefault ?? false,
    reasoningEnabled: retained ? config?.reasoningEnabled ?? null : null,
    thinkingBudget: retained ? config?.thinkingBudget ?? null : null,
    reasoningEffort: retained ? config?.reasoningEffort ?? null : null,
    maxIterations: config?.maxIterations ?? null, summarizationModel: null, summarizationProvider: null,
    imageGenerationModel: null, subagentAllowedTools: config?.subagentAllowedTools ?? null,
    subagentAllowedSkillIds: config?.subagentAllowedSkillIds,
    subagentMaxParallel: config?.subagentMaxParallel,
    subagentMaxCallsPerTurn: config?.subagentMaxCallsPerTurn,
    subagentTokenBudget: config?.subagentTokenBudget,
    delegationLimitsV2: config?.delegationLimitsV2,
    providerStreaming: config?.providerStreaming,
  };
}
