import ttsProviderPresets from "../../../../shared/tts-provider-presets.json";
import {
  attachModelDescriptors,
  canonicalModelProviderId,
  inferModelCatalogRegion,
  modelEndpointId,
  normalizeModelEndpointUrl,
  selectImplicitDefault,
  type LegacyCatalogModel,
  type ModelDescriptor,
} from './modelCatalog';

export interface TtsCatalogItem {
  id: string;
  name: string;
  recommended?: boolean;
  modelIds?: string[];
  languages?: string[];
  gender?: string | null;
  description?: string | null;
  previewUrl?: string | null;
  maxInputCharacters?: number;
  descriptor?: ModelDescriptor;
}

export interface TtsProviderPreset {
  id: string;
  name: string;
  provider: string;
  apiStyle: string;
  requiresApiKey: boolean;
  local?: boolean;
  baseUrl: string;
  description: string;
  models: TtsCatalogItem[];
  voices: TtsCatalogItem[];
  outputFormats: string[];
  speedRange?: [number, number];
  lastVerifiedAt: string;
  documentationUrls: string[];
}

type RawTtsProviderPreset = Omit<TtsProviderPreset, 'models'> & { models: LegacyCatalogModel[] };

export const TTS_PROVIDER_PRESETS: TtsProviderPreset[] =
  (ttsProviderPresets as RawTtsProviderPreset[]).map((preset) => ({
    ...preset,
    models: attachModelDescriptors(preset.models, {
      surface: 'text_to_speech',
      providerId: canonicalModelProviderId(preset.id, preset.provider),
      endpointId: modelEndpointId('text_to_speech', preset.id),
      region: inferModelCatalogRegion(preset.baseUrl),
      apiStyle: preset.apiStyle,
      outputFormats: preset.outputFormats,
    }) as TtsCatalogItem[],
  }));

export function defaultTtsItem(items: TtsCatalogItem[]): TtsCatalogItem | null {
  const models = items.filter(
    (item): item is TtsCatalogItem & { descriptor: ModelDescriptor } => Boolean(item.descriptor),
  );
  if (models.length === items.length) {
    // Choosing a provider preset is an explicit settings action. Its documented
    // default can be selected before the account is probed; this must not label
    // the model callable or broaden the agent's implicit model policy.
    const eligible = models.filter(({ descriptor }) => descriptor.lifecycle === 'active'
      && descriptor.access === 'public' && descriptor.availableToCredential !== false);
    return selectImplicitDefault(models)
      ?? eligible.find((item) => item.recommended) ?? eligible[0] ?? null;
  }
  // Voice rows are not model descriptors and keep their existing preference order.
  return items.find((item) => item.recommended) ?? items[0] ?? null;
}

export function ttsVoiceSupportsModel(voice: TtsCatalogItem, model: string): boolean {
  return !voice.modelIds?.length || voice.modelIds.includes(model.trim());
}

/** Change a known incompatible preset voice with the model, but retain private
 * voice IDs. A custom-only model deliberately has no automatic system voice. */
export function ttsVoiceForModel(preset: TtsProviderPreset, model: string, currentVoice = '', baseUrl: string | null = preset.baseUrl): string {
  if (currentVoice.trim() && !ttsPresetEndpointMatches(preset, baseUrl)) return currentVoice;
  const current = preset.voices.find((voice) => voice.id === currentVoice.trim());
  if (currentVoice.trim() && (!current || ttsVoiceSupportsModel(current, model))) return currentVoice;
  return defaultTtsItem(preset.voices.filter((voice) => ttsVoiceSupportsModel(voice, model)))?.id ?? '';
}

export function ttsSpeedRange(preset: TtsProviderPreset, baseUrl: string | null = preset.baseUrl): [number, number] {
  return ttsPresetEndpointMatches(preset, baseUrl) ? preset.speedRange ?? [0.5, 2] : [0.5, 2];
}

export function ttsPresetEndpointMatches(preset: TtsProviderPreset, baseUrl: string | null): boolean {
  if (preset.local) return true;
  const effectiveBase = baseUrl?.trim() || TTS_PROVIDER_PRESETS.find(candidate => candidate.apiStyle === preset.apiStyle)?.baseUrl || 'https://api.openai.com/v1';
  if (preset.baseUrl && normalizeModelEndpointUrl(effectiveBase) === normalizeModelEndpointUrl(preset.baseUrl)) return true;
  if (preset.apiStyle !== 'dashscope_speech') return false;
  try {
    const url = new URL(effectiveBase);
    const host = url.hostname;
    const official = ['dashscope.aliyuncs.com', 'dashscope-intl.aliyuncs.com'].includes(host)
      || ['.cn-beijing.maas.aliyuncs.com', '.ap-southeast-1.maas.aliyuncs.com'].some(suffix => host.endsWith(suffix)
        && /^[a-z0-9-]+$/i.test(host.slice(0, -suffix.length)));
    return official && ['https:', 'wss:'].includes(url.protocol) && !url.port
      && !url.username && !url.password && !url.search && !url.hash
      && ['/api-ws/v1/inference', '/api/v1/services/audio/tts', '/api/v1/services/audio/tts/SpeechSynthesizer'].includes(url.pathname.replace(/\/+$/, ''));
  } catch { return false; }
}

/** Resolve the catalog entry that backs a saved text-to-speech configuration. */
export function findTtsProviderPreset(config: {
  provider: string;
  apiStyle: string;
} | null | undefined): TtsProviderPreset | null {
  if (!config) return null;
  return TTS_PROVIDER_PRESETS.find(
    (preset) => preset.provider === config.provider && preset.apiStyle === config.apiStyle,
  ) ?? null;
}
