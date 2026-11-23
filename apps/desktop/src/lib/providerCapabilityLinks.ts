import type { CapabilityLinkOption } from '../components/settings/CapabilityLinkDialog';
import type { TranslationKey } from '../i18n/types';
import type { AgentConfig, AppConfig, ImageGenerationConfig, SpeechToTextConfig, TextToSpeechConfig, SystemOneConfig } from '../types/conversation';
import type { EmbedderConfig } from '../types/embedder';
import { savedConnectionModelFeatures } from './connectionModelFeatures';
import type { ProviderModelCatalogSnapshot } from './providerModelCatalog';
import { credentialFeatureEndpoint } from './providerCredentials';
import { IMAGE_PROVIDER_PRESETS, imageConfigFromPreset } from './imageProviderPresets';
import { STT_PROVIDER_PRESETS, defaultSttItem } from './sttProviderPresets';
import { TTS_PROVIDER_PRESETS, defaultTtsItem, ttsSpeedRange, ttsVoiceForModel } from './ttsProviderPresets';
import { EMBEDDING_PROVIDER_PRESETS, defaultEmbeddingModel } from './embeddingProviderPresets';
import { SYSTEM_ONE_PROVIDERS, DEFAULT_SYSTEM_ONE_CONFIG, changeSystemOneProvider } from './systemOneProviders';
import { deepSeekSearchProvider } from './webSearchConnections';
import videos from '../../../../shared/video-provider-presets.json';

export interface CapabilityPatch {
  imageGeneration?: ImageGenerationConfig;
  speechToText?: SpeechToTextConfig;
  textToSpeech?: TextToSpeechConfig;
  embedding?: EmbedderConfig;
  systemOne?: SystemOneConfig;
  deepSeekSearch?: boolean;
  nativeSearch?: boolean;
}

type Translate = (key: TranslationKey, params?: Record<string, string | number>) => string;
export type CapabilityRoute = CapabilityLinkOption & { patch?: CapabilityPatch };
interface Setting {
  connectionId?: string | null;
  apiKey?: string;
  provider?: string;
  model?: string;
  baseUrl?: string | null;
}

export function providerCapabilityLinks(source: AgentConfig, app: AppConfig, embed: EmbedderConfig, t: Translate, catalog?: ProviderModelCatalogSnapshot | null): CapabilityRoute[] {
  const routes: CapabilityRoute[] = [];
  const binding = { connectionId: source.id, apiKey: '' };
  function add(group: string, id: string, name: string, label: TranslationKey, endpoint: string, model: string,
    previous: Setting | undefined, patch: CapabilityPatch, extra = '') {
    const linked = previous?.connectionId === source.id && previous.baseUrl === endpoint;
    const replacing = previous && !linked && (previous.connectionId || previous.apiKey || previous.model);
    const note = linked ? t('providers.capabilityLinked') : replacing
      ? t('providers.capabilityReplaces', { name: previous.model || previous.provider || '' }) : '';
    routes.push({ id: group + '/' + id, group, label: t(label) + ' · ' + name, linked,
      detail: [model || t('providers.capabilityModel'), endpoint, note, extra].filter(Boolean).join(' · '), patch });
  }
  for (const preset of IMAGE_PROVIDER_PRESETS) {
    const endpoint = credentialFeatureEndpoint(source, 'image', preset);
    if (!endpoint) continue;
    const current = app.imageGeneration;
    const initial: ImageGenerationConfig = current ?? { provider: '', apiStyle: '', model: '', apiKey: '', baseUrl: null, quality: null, size: null, outputFormat: null };
    const config = imageConfigFromPreset(initial, preset);
    if (current?.baseUrl === endpoint && current.apiStyle === preset.apiStyle && current.model.trim()
      && preset.models.some(row => row.id === current.model && row.descriptor.lifecycle !== 'removed')) Object.assign(config, current);
    add('image', preset.id, preset.name, 'settings.imageGeneration', endpoint, config.model,
      current?.apiStyle === preset.apiStyle ? current : { ...current, connectionId: null },
      { imageGeneration: { ...config, source: 'apiKey', baseUrl: endpoint, ...binding } });
  }
  for (const preset of STT_PROVIDER_PRESETS) {
    const endpoint = credentialFeatureEndpoint(source, 'stt', preset);
    if (!endpoint) continue;
    const current = app.speechToText;
    const same = current?.baseUrl === endpoint && current.apiStyle === preset.apiStyle;
    const model = (same && current.model) || defaultSttItem(preset.models)?.id || '';
    add('stt', preset.id, preset.name, 'settings.speechToText', endpoint, model, same ? current : { ...current, connectionId: null },
      { speechToText: { ...current, ...binding, provider: preset.provider, apiStyle: preset.apiStyle, baseUrl: endpoint, model, language: current?.language ?? null } });
  }
  for (const preset of TTS_PROVIDER_PRESETS) {
    const endpoint = credentialFeatureEndpoint(source, 'tts', preset);
    if (!endpoint) continue;
    const current = app.textToSpeech;
    const same = current?.baseUrl === endpoint && current.apiStyle === preset.apiStyle;
    const model = (same && current.model) || defaultTtsItem(preset.models)?.id || '';
    const voice = ttsVoiceForModel(preset, model, same ? current.voice : '', endpoint);
    const [min, max] = ttsSpeedRange(preset, endpoint, model);
    add('tts', preset.id, preset.name, 'settings.textToSpeech', endpoint, model, same ? current : { ...current, connectionId: null },
      { textToSpeech: { ...current, ...binding, provider: preset.provider, apiStyle: preset.apiStyle, baseUrl: endpoint, model, voice,
        outputFormat: (same && current.outputFormat) || preset.outputFormats[0] || 'mp3', speed: Math.min(max, Math.max(min, current?.speed ?? 1)) } });
  }
  for (const preset of EMBEDDING_PROVIDER_PRESETS) {
    const endpoint = credentialFeatureEndpoint(source, 'embedding', preset);
    if (!endpoint) continue;
    const same = embed.provider === 'api' && embed.apiBaseUrl === endpoint;
    const model = (same && preset.models.find(row => row.id === embed.apiModel && row.descriptor.lifecycle !== 'removed')) || defaultEmbeddingModel(preset);
    const apiModel = model?.id ?? '';
    add('embedding', preset.id, preset.name, 'providers.capabilityEmbedding', endpoint, apiModel,
      { ...embed, baseUrl: embed.apiBaseUrl, model: embed.provider === 'api' ? embed.apiModel : embed.provider, connectionId: same ? embed.connectionId : null },
      { embedding: { ...embed, ...binding, provider: 'api', apiBaseUrl: endpoint, apiModel,
        vectorDimensions: same && embed.apiModel === apiModel ? embed.vectorDimensions : model?.dimensions ?? embed.vectorDimensions } },
      !same || apiModel !== embed.apiModel ? t('providers.capabilityReindex') : '');
  }
  for (const preset of SYSTEM_ONE_PROVIDERS) {
    const endpoint = credentialFeatureEndpoint(source, 'systemOne', preset);
    if (!endpoint) continue;
    const current = app.systemOne ?? DEFAULT_SYSTEM_ONE_CONFIG;
    const config = changeSystemOneProvider({ ...current, provider: '' }, preset.id, [source]);
    const model = (current.provider === preset.id && preset.models.some(row => row.id === current.model && row.status !== 'removed') && current.model) || config.model;
    add('systemOne', preset.id, preset.name, 'settings.systemOneTitle', endpoint, model,
      current.enabled ? current : undefined, { systemOne: { ...config, ...binding, enabled: true, baseUrl: endpoint, model } });
  }
  const search = credentialFeatureEndpoint(source, 'webSearch', { ...deepSeekSearchProvider, provider: 'deep_seek' });
  if (search && app.webSearch) {
    const current = app.webSearch.customProviders.find(row => row.preset === 'deepseek');
    add('search', 'deepseek', deepSeekSearchProvider.name, 'settings.webSearchTitle', search, 'Search & Fetch',
      current?.enabled && app.webSearch.executionMode === 'nexaRouter' && app.webSearch.providerMode === 'custom_first' ? current : undefined, { deepSeekSearch: true });
  }
  for (const preset of videos) {
    const endpoint = credentialFeatureEndpoint(source, 'video', preset);
    if (!endpoint) continue;
    const linked = preset.providerId === 'openrouter' && source.provider === 'openrouter';
    routes.push({ id: 'video/' + preset.id, label: t('providers.capabilityVideo') + ' · ' + preset.name,
      detail: [linked ? t('providers.capabilityAvailable') : t('providers.capabilityCatalogOnly'), endpoint].join(' · '), linked, disabled: true });
  }
  const selected = source.model.trim().replace(/^models\//, '').toLowerCase();
  const descriptor = [...(catalog?.tombstones ?? []), ...(catalog?.descriptors ?? [])].find(row =>
    [row.id, ...row.aliases].some(id => id.toLowerCase() === selected));
  const features = savedConnectionModelFeatures({ ...source, endpointId: source.providerEndpointId ?? catalog?.endpointId }, 'desktop', descriptor);
  for (const feature of features) {
    if (feature.id === 'chat') continue;
    if (feature.id === 'nativeWebSearch') {
      const linked = app.webSearch?.executionMode !== 'nexaRouter';
      routes.push({ id: 'search/native', group: 'search', label: t(feature.labelKey), linked,
        detail: linked ? t('providers.capabilityAvailable') : t('providers.capabilityReplaces', { name: t('settings.webSearchExecutionMode.nexaRouter') }),
        patch: { nativeSearch: true } });
      continue;
    }
    routes.push({ id: 'model/' + feature.id, label: t(feature.labelKey), linked: true,
      detail: [t('providers.capabilityAvailable'), feature.detailKey && t(feature.detailKey)].filter(Boolean).join(' · ') });
  }
  return routes;
}
