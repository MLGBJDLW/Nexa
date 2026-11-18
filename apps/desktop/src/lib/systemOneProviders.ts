import catalog from '../../../../shared/system-one-provider-presets.json';
import type { SystemOneConfig } from '../types/conversation';

export const SYSTEM_ONE_PROVIDERS = catalog;
export const DEFAULT_SYSTEM_ONE_CONFIG: SystemOneConfig = {
  enabled: false, provider: 'typesafe', apiKey: '', model: '', baseUrl: null,
};

export function systemOneModelRemoved(provider: string, model: string): boolean {
  return (provider === 'siliconflow-cn' && model === 'diffusiongemma')
    || (catalog.find(preset => preset.id === provider)?.models.some(row => row.id === model && row.status === 'removed') ?? false);
}

export function changeSystemOneProvider(config: SystemOneConfig, provider: string): SystemOneConfig {
  const preset = SYSTEM_ONE_PROVIDERS.find(item => item.id === provider);
  if (!preset || preset.id === config.provider) return config;
  const publicModels = preset.models.filter(model => model.access === 'public');
  const model = publicModels.find(model => model.status === 'active' && 'recommended' in model && model.recommended)
    ?? publicModels.find(model => model.status === 'active')
    ?? publicModels.find(model => model.status === 'preview');
  return { ...config, provider: preset.id, model: model?.id ?? '', baseUrl: preset.baseUrl || null, apiKey: '' };
}

export function systemOneConfigured(config: SystemOneConfig): boolean {
  const preset = SYSTEM_ONE_PROVIDERS.find(item => item.id === config.provider);
  return Boolean(preset && config.apiKey.trim() && config.model.trim() && !systemOneModelRemoved(config.provider, config.model) && (config.baseUrl?.trim() || preset.baseUrl));
}
