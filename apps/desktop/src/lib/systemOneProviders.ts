import catalog from '../../../../shared/system-one-provider-presets.json';
import type { SystemOneConfig } from '../types/conversation';

export const SYSTEM_ONE_PROVIDERS = catalog;
export const DEFAULT_SYSTEM_ONE_CONFIG: SystemOneConfig = {
  enabled: false, provider: 'typesafe', apiKey: '', model: 'jev-latest', baseUrl: null,
};

export function systemOneModelRemoved(provider: string, model: string): boolean {
  return (provider === 'siliconflow-cn' && model === 'diffusiongemma')
    || (catalog.find(preset => preset.id === provider)?.models.some(row => row.id === model && row.status === 'removed') ?? false);
}

export function changeSystemOneProvider(config: SystemOneConfig, provider: string): SystemOneConfig {
  const preset = SYSTEM_ONE_PROVIDERS.find(item => item.id === provider);
  if (!preset || preset.id === config.provider) return config;
  return { ...config, provider: preset.id, model: preset.models[0]?.id ?? '', baseUrl: preset.baseUrl || null, apiKey: '' };
}

export function systemOneConfigured(config: SystemOneConfig): boolean {
  const preset = SYSTEM_ONE_PROVIDERS.find(item => item.id === config.provider);
  return Boolean(preset && config.apiKey.trim() && config.model.trim() && !systemOneModelRemoved(config.provider, config.model) && (config.baseUrl?.trim() || preset.baseUrl));
}
