import type { TranslationKey } from '../i18n/types';
import { modelEndpointId, type ModelDescriptor } from './modelCatalog';
import { findProviderPreset, isChatModelId, isRemovedProviderModel } from './providerPresets';

interface SavedChatConnection {
  provider: string;
  baseUrl?: string | null;
  model: string;
  endpointId?: string | null;
}

export interface ConnectionModelFeature {
  id: 'chat' | 'imageInput' | 'nativeWebSearch' | 'conversationSummary' | 'incrementalLive' | 'nativeLive';
  labelKey: TranslationKey;
  detailKey?: TranslationKey;
}

/**
 * Project the features of a saved chat route. Pass an account-specific descriptor
 * only from the current connection's catalog. A custom discovery endpoint ID is
 * opaque and must match the identity stored with that same connection.
 */
export function savedConnectionModelFeatures(
  connection: SavedChatConnection,
  platform: 'desktop' | 'phone',
  descriptor?: ModelDescriptor | null,
): ConnectionModelFeature[] {
  const provider = connection.provider.trim();
  // Legacy Qwen without an explicit URL connects to Beijing PAYG in core.
  const preset = findProviderPreset(provider === 'qwen' && !connection.baseUrl?.trim()
    ? { provider: 'alibaba_model_studio' }
    : connection);
  if (preset?.runtime || preset?.retired || provider.startsWith('acp_')
    || provider === 'github_copilot' || provider === 'openai_codex') return [];

  const features: ConnectionModelFeature[] = [];
  if (platform === 'desktop' && preset
    && ['openai', 'google', 'alibaba-model-studio', 'qwen-cloud-intl'].includes(preset.id)) {
    // Live selects a separate realtime model on this saved connection.
    features.push({ id: 'nativeLive', labelKey: 'live.title', detailKey: 'live.nativeHint' });
  }

  const rawModel = connection.model.trim();
  const model = (provider === 'google' || provider === 'vertex' ? rawModel.replace(/^models\//, '') : rawModel).toLowerCase();
  if (!provider || !isChatModelId(model) || (preset && isRemovedProviderModel(preset.id, model))) return features;
  const matchesModel = (candidate: ModelDescriptor) =>
    [candidate.id, ...candidate.aliases].some(id => id.toLowerCase() === model);
  const bundled = preset?.models.find(candidate => matchesModel(candidate.descriptor))?.descriptor;
  const builtinEndpoint = preset && modelEndpointId('text', preset.id);
  const discoveredEndpoint = connection.endpointId?.startsWith('text:custom-') ? connection.endpointId : null;
  const account = descriptor && descriptor.endpointKinds.includes('text') && matchesModel(descriptor)
    && descriptor.endpointIds.some(id => id === builtinEndpoint || id === discoveredEndpoint)
    ? descriptor : null;
  const selected = account ?? bundled;
  if (selected?.lifecycle === 'removed' || selected?.availableToCredential === false) return features;

  features.unshift({ id: 'chat', labelKey: 'chat.title' });
  if (selected && (selected.capabilities.vision ?? selected.inputModalities.includes('image'))) {
    features.push({ id: 'imageInput', labelKey: 'providers.tagVision' });
  }
  // Mirror NativeSearchPlan::for_preset. Runtime support is per exact model ID
  // except for the endpoint-wide OpenRouter server tool and xAI Responses.
  if ((bundled?.id.toLowerCase() === model && bundled.capabilities.nativeWebSearch)
    || preset?.nativeWebSearch?.dialect === 'openRouterServerTool'
    || preset?.nativeWebSearch?.dialect === 'xaiResponses') {
    features.push({
      id: 'nativeWebSearch', labelKey: 'settings.webSearchNativeSupport', detailKey: 'settings.webSearchNativeSupport.desc',
    });
  }
  if (platform === 'desktop') {
    features.push(
      { id: 'conversationSummary', labelKey: 'usage.operation.summarization', detailKey: 'settings.summarizationHelp' },
      { id: 'incrementalLive', labelKey: 'live.incremental', detailKey: 'live.incrementalHint' },
    );
  }
  return features;
}
