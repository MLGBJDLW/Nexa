import { invoke } from '@tauri-apps/api/core';
import { accountSnapshotRequest } from './accountSnapshotRequest';
import type { CopilotModelSummary, TerminalSessionInfo } from './api';
import { invalidateSubscriptionModels, runtimeCatalogKey } from './subscriptionModelCatalog';
import type { AgentConfig, SaveAgentConfigInput } from '../types/conversation';
import type { ExternalAccountUsage } from './subscriptionUsage';

export const getExternalAgentUsage = (agentConfigId: string, model: string, conversationId?: string | null, projectId?: string | null) =>
  accountSnapshotRequest(JSON.stringify(['external', agentConfigId, model, conversationId, projectId]), () =>
    invoke<ExternalAccountUsage>('get_external_agent_usage_cmd', { agentConfigId, model, conversationId, projectId }));

export interface ExternalAgentLaunch {
  executable: string | null;
  args?: string[] | null;
  env?: Record<string, string>;
  workingDirectory: string;
  configOptions?: Record<string, string>;
  configOptionsModel?: string;
  mcpServerIds?: string[];
  authenticationMethod?: string;
}

export function parseExternalLaunchFields(argsText: string, envText: string): Pick<ExternalAgentLaunch, 'args' | 'env'> {
  try {
    const args: unknown = argsText.trim() ? JSON.parse(argsText) : null;
    const env: unknown = envText.trim() ? JSON.parse(envText) : {};
    if (args !== null && (!Array.isArray(args) || !args.every(value => typeof value === 'string' && !value.includes('\0')))) throw new Error();
    if (env === null || typeof env !== 'object' || Array.isArray(env)
      || !Object.entries(env).every(([key,value]) => key.length > 0 && !/[=\0]/.test(key) && typeof value === 'string' && !value.includes('\0'))) throw new Error();
    return { args: args as string[] | null, env: env as Record<string,string> };
  } catch { throw new Error('Use a JSON string array for arguments and a JSON object of string values for environment variables.'); }
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
  authMethods?: { id: string; name: string; description?: string; kind: 'agent' | 'terminal' }[];
  requiresAuthentication?: boolean;
  setupError?: string;
}
export const inspectExternalAgent = (provider: string, launch: ExternalAgentLaunch, model?: string, requestId?: string) =>
  invoke<ExternalAgentCatalog>('inspect_external_agent_cmd', { provider, launch, model, requestId });

export const authenticateExternalAgent = (provider: string, launch: ExternalAgentLaunch, methodId: string, model?: string, requestId?: string) =>
  invoke<{ catalog?: ExternalAgentCatalog; terminal?: TerminalSessionInfo }>('authenticate_external_agent_cmd', { provider, launch, methodId, model, requestId });
export const cancelExternalAgentSetup = (requestId: string) => invoke<void>('cancel_external_agent_setup_cmd', { requestId });

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
