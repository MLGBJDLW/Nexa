import sttProviderPresets from '../../../../shared/stt-provider-presets.json';
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

export interface SttCatalogItem {
  id: string;
  name: string;
  recommended?: boolean;
  descriptor: ModelDescriptor;
}

export type SttAudioInput = 'completeFile' | 'chunkStream';
export type SttTranscriptDelivery = 'finalOnly' | 'interimAndFinal';
export type SttInterimSemantics = 'none' | 'appendDelta' | 'replaceSnapshot';
export type SttFinalization = 'endOfFile' | 'clientCommit' | 'sessionFinish' | 'taskFinish';
export type SttTransport = 'httpMultipart' | 'httpJson' | 'websocket' | 'localOffline';

export interface SttRuntimeCapabilities {
  audioInput: SttAudioInput;
  transcriptDelivery: SttTranscriptDelivery;
  interimSemantics: SttInterimSemantics;
  finalization: SttFinalization;
  transport: SttTransport;
  sampleRateHz: number;
  /** Upstream engine fact; not a claim that Nexa's selected adapter is live. */
  engineStreamingCapable: boolean;
}

export const FINAL_ONLY_STT_CAPABILITIES: SttRuntimeCapabilities = {
  audioInput: 'completeFile',
  transcriptDelivery: 'finalOnly',
  interimSemantics: 'none',
  finalization: 'endOfFile',
  transport: 'httpMultipart',
  sampleRateHz: 16_000,
  engineStreamingCapable: false,
};

export interface SttProviderPreset {
  id: string;
  name: string;
  provider: string;
  apiStyle: string;
  requiresApiKey: boolean;
  local?: boolean;
  baseUrl: string;
  sherpaModelFamily?: string;
  description: string;
  transcription: SttRuntimeCapabilities;
  models: SttCatalogItem[];
  lastVerifiedAt: string;
  documentationUrls: string[];
}

type RawSttProviderPreset = Omit<SttProviderPreset, 'models'> & { models: LegacyCatalogModel[] };

export const STT_PROVIDER_PRESETS: SttProviderPreset[] =
  (sttProviderPresets as RawSttProviderPreset[]).map((preset) => ({
    ...preset,
    models: attachModelDescriptors(preset.models, {
      surface: 'speech_to_text',
      providerId: canonicalModelProviderId(preset.id, preset.provider),
      endpointId: modelEndpointId('speech_to_text', preset.id),
      region: inferModelCatalogRegion(preset.baseUrl),
      apiStyle: preset.apiStyle,
    }) as SttCatalogItem[],
  }));

export function defaultSttItem(items: SttCatalogItem[]): SttCatalogItem | null {
  // Selecting a provider is an explicit settings action, just like TTS. A
  // documented public preview can be chosen without claiming account access.
  const eligible = items.filter(({ descriptor }) => (descriptor.lifecycle === 'active' || descriptor.lifecycle === 'preview')
    && descriptor.access === 'public' && descriptor.availableToCredential !== false);
  return selectImplicitDefault(items)
    ?? eligible.find((item) => item.recommended) ?? null;
}

/** Resolve the catalog entry that backs a saved speech-to-text configuration. */
export function findSttProviderPreset(config: {
  provider: string;
  apiStyle: string;
  baseUrl?: string | null;
  sherpaModelFamily?: string | null;
} | null | undefined): SttProviderPreset | null {
  if (!config) return null;
  const sherpaFamily = config.apiStyle === 'sherpa_onnx'
    ? config.sherpaModelFamily ?? 'sense_voice'
    : null;
  const matches = STT_PROVIDER_PRESETS.filter((preset) =>
    preset.provider === config.provider
    && preset.apiStyle === config.apiStyle
    && (preset.sherpaModelFamily ?? null) === sherpaFamily,
  );
  // An explicit proxy still uses this provider's protocol. The connection
  // binding independently verifies its destination before supplying a key.
  const endpoint = normalizeModelEndpointUrl(config.baseUrl);
  return matches.find(preset => normalizeModelEndpointUrl(preset.baseUrl) === endpoint) ?? matches[0] ?? null;
}

/** Product/runtime capability for a concrete saved configuration. Unknown and
 * custom endpoints are final-only until an explicit dialect proves otherwise. */
export function sttRuntimeCapabilities(config: {
  provider: string;
  apiStyle: string;
  model?: string;
  sherpaModelFamily?: string | null;
} | null | undefined): SttRuntimeCapabilities {
  const preset = findSttProviderPreset(config);
  if (!preset) return FINAL_ONLY_STT_CAPABILITIES;
  if (
    preset.transcription.transcriptDelivery === 'interimAndFinal'
    && !preset.models.some((model) => model.id === config?.model?.trim())
  ) {
    return FINAL_ONLY_STT_CAPABILITIES;
  }
  return preset.transcription;
}
