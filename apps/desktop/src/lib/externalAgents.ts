import { invoke } from '@tauri-apps/api/core';
import type { CopilotModelSummary } from './api';
import { invalidateSubscriptionModels } from './subscriptionModelCatalog';
import type { AgentConfig, SaveAgentConfigInput } from '../types/conversation';

export interface ExternalAgentLaunch {
  executable: string | null;
  workingDirectory: string;
}

export const getExternalAgentLaunch = (provider: string) =>
  invoke<ExternalAgentLaunch>('get_external_agent_launch_cmd', { provider });
export const probeExternalAgent = (provider: string, launch: ExternalAgentLaunch) =>
  invoke<CopilotModelSummary[]>('probe_external_agent_cmd', { provider, launch });
export async function saveExternalAgentProfile(config: SaveAgentConfigInput, launch: ExternalAgentLaunch): Promise<AgentConfig> {
  const result = await invoke<AgentConfig>('save_external_agent_profile_cmd', { config, launch });
  invalidateSubscriptionModels(config.provider);
  return result;
}
