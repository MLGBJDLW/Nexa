import { invoke } from '@tauri-apps/api/core';
import type { CopilotModelSummary } from './api';
import { invalidateSubscriptionModels, runtimeCatalogKey } from './subscriptionModelCatalog';
import type { AgentConfig, SaveAgentConfigInput } from '../types/conversation';

export interface ExternalAgentLaunch {
  executable: string | null;
  workingDirectory: string;
  configOptions?: Record<string, string>;
  configOptionsModel?: string;
  mcpServerIds?: string[];
}

export interface ExternalAgentConfigOption {
  id: string;
  name: string;
  category: string | null;
  currentValue: string;
  options: { value: string; name: string }[];
}
export interface ExternalAgentCatalog {
  models: CopilotModelSummary[];
  configOptions: ExternalAgentConfigOption[];
  commands: string[];
}
export const inspectExternalAgent = (provider: string, launch: ExternalAgentLaunch, model?: string) =>
  invoke<ExternalAgentCatalog>('inspect_external_agent_cmd', { provider, launch, model });

export const getExternalAgentLaunch = (agentConfigId?: string) => agentConfigId
  ? invoke<ExternalAgentLaunch>('get_external_agent_launch_cmd', { agentConfigId })
  : Promise.resolve<ExternalAgentLaunch>({ executable: null, workingDirectory: '' });
export const probeExternalAgent = (provider: string, launch: ExternalAgentLaunch) =>
  invoke<CopilotModelSummary[]>('probe_external_agent_cmd', { provider, launch });
export async function saveExternalAgentProfile(config: SaveAgentConfigInput, launch: ExternalAgentLaunch): Promise<AgentConfig> {
  const result = await invoke<AgentConfig>('save_external_agent_profile_cmd', { config, launch });
  invalidateSubscriptionModels(runtimeCatalogKey(config.provider, result.id));
  return result;
}
