import catalog from '../../../../shared/system-one-provider-presets.json';
import type { SystemOneConfig } from '../types/conversation';
import { alibabaWorkspaceRegion, normalizeModelEndpointUrl } from './modelCatalog';
import { findSharedProviderCredential, type ProviderCredentialConfig } from './providerCredentials';

export const SYSTEM_ONE_PROVIDERS = catalog;
export const DEFAULT_SYSTEM_ONE_CONFIG: SystemOneConfig = {
  enabled: false, provider: 'typesafe', apiKey: '', model: '', baseUrl: null,
};

export function systemOneModelRemoved(provider: string, model: string): boolean {
  return (provider === 'siliconflow-cn' && model === 'diffusiongemma')
    || (catalog.find(preset => preset.id === provider)?.models.some(row => row.id === model && row.status === 'removed') ?? false);
}

const workspaceRegions: Readonly<Record<string, string>> = {
  'alibaba-model-studio-cn': 'cn-beijing',
  'alibaba-model-studio-intl': 'ap-southeast-1',
};

export function systemOneBaseUrl(config: SystemOneConfig): string | null {
  const preset = SYSTEM_ONE_PROVIDERS.find(item => item.id === config.provider);
  if (!preset) return null;
  const base = config.baseUrl?.trim() || preset.baseUrl;
  const region = workspaceRegions[preset.id];
  if (region) return alibabaWorkspaceRegion(base) === region ? normalizeModelEndpointUrl(base) : null;
  try {
    const url = new URL(base);
    return !url.username && !url.password && normalizeModelEndpointUrl(base) === preset.baseUrl ? preset.baseUrl : null;
  } catch { return null; }
}

export function changeSystemOneProvider(config: SystemOneConfig, provider: string, connections: ProviderCredentialConfig[] = []): SystemOneConfig {
  const preset = SYSTEM_ONE_PROVIDERS.find(item => item.id === provider);
  if (!preset || preset.id === config.provider) return config;
  const publicModels = preset.models.filter(model => model.access === 'public');
  const model = publicModels.find(model => model.status === 'active' && 'recommended' in model && model.recommended)
    ?? publicModels.find(model => model.status === 'active')
    ?? publicModels.find(model => model.status === 'preview');
  const workspaces = workspaceRegions[preset.id]
    ? connections.filter(source => source.apiKey.trim() && alibabaWorkspaceRegion(source.baseUrl ?? '') === workspaceRegions[preset.id]) : [];
  const workspace = workspaces.find(source => source.isDefault)
    ?? (new Set(workspaces.map(source => normalizeModelEndpointUrl(source.baseUrl))).size === 1 ? workspaces[0] : null);
  return { ...config, provider: preset.id, model: model?.id ?? '', baseUrl: preset.baseUrl || workspace?.baseUrl || null, apiKey: '' };
}

export function systemOneCredential(config: SystemOneConfig, connections: ProviderCredentialConfig[]) {
  const endpoint = systemOneBaseUrl(config);
  return endpoint ? findSharedProviderCredential(connections, config.provider, endpoint) : null;
}

export function systemOneConfigured(config: SystemOneConfig, inheritedKey: boolean): boolean {
  return Boolean(systemOneBaseUrl(config) && (config.apiKey.trim() || inheritedKey)
    && config.model.trim() && !systemOneModelRemoved(config.provider, config.model));
}
