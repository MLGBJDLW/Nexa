import catalog from '../../../../shared/system-one-provider-presets.json';
import type { SystemOneConfig } from '../types/conversation';

export const SYSTEM_ONE_PROVIDERS = catalog;
export const DEFAULT_SYSTEM_ONE_CONFIG: SystemOneConfig = {
  enabled: false, provider: 'typesafe', apiKey: '', model: 'jev-latest', baseUrl: null,
};

export function changeSystemOneProvider(config: SystemOneConfig, provider: string): SystemOneConfig {
  const preset = SYSTEM_ONE_PROVIDERS.find(item => item.id === provider);
  if (!preset || preset.id === config.provider) return config;
  return { ...config, provider: preset.id, model: preset.models[0]?.id ?? '', baseUrl: preset.baseUrl || null, apiKey: '' };
}

export function systemOneConfigured(config: SystemOneConfig): boolean {
  const preset = SYSTEM_ONE_PROVIDERS.find(item => item.id === config.provider);
  return Boolean(preset && config.apiKey.trim() && config.model.trim() && (config.baseUrl?.trim() || preset.baseUrl));
}
