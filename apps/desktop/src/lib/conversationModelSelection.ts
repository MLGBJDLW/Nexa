import type { AgentConfig, Conversation } from '../types/conversation';
import type { TurnModelSelection } from '../features/models/modelChoices';

export function modelSelectionForConfig(config: AgentConfig | null): TurnModelSelection | null {
  return config ? { model: config.model, reasoningEnabled: config.reasoningEnabled,
    reasoningEffort: config.reasoningEffort, thinkingBudget: config.thinkingBudget } : null;
}

/** Credentials refresh independently of the chat's persisted model choice. */
export function findConfigForConversation(configs: AgentConfig[], conversation: Conversation, fallback: AgentConfig | null): AgentConfig | null {
  const config = conversation.agentConfigId
    ? configs.find(config => config.id === conversation.agentConfigId && config.provider === conversation.provider)
    : configs.find(config => config.provider === conversation.provider && config.model === conversation.model && config.isDefault)
      ?? configs.find(config => config.provider === conversation.provider && config.model === conversation.model)
      ?? configs.find(config => config.provider === conversation.provider && config.id === fallback?.id)
      ?? configs.find(config => config.provider === conversation.provider);
  if (!config) return null;
  return { ...config, ...(conversation.modelSelection ?? {}), model: conversation.model, modelId: conversation.model };
}

export function refreshSelectedConfig(configs: AgentConfig[], selected: AgentConfig | null): AgentConfig | null {
  const saved = configs.find(config => config.id === selected?.id && config.provider === selected.provider);
  return saved && selected ? { ...saved, ...modelSelectionForConfig(selected), modelId: selected.model } : null;
}
