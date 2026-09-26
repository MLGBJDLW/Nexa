import embeddingProviderPresets from "../../../../shared/embedding-provider-presets.json";
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

export interface EmbeddingModelPreset {
  id: string;
  name: string;
  dimensions: number;
  supportsDimensionOverride: boolean;
  allowedDimensions?: number[];
  minDimensions?: number;
  maxDimensions?: number;
  maxBatchSize?: number;
  recommended?: boolean;
  descriptor: ModelDescriptor;
}

export interface EmbeddingProviderPreset {
  id: string;
  name: string;
  provider: string;
  baseUrl: string;
  description: string;
  apiStyle?: 'openai_embeddings' | 'voyage_embeddings' | 'cohere_embeddings' | 'gemini_embeddings' | 'jina_embeddings';
  maxBatchSize?: number;
  requiresApiKey?: boolean;
  models: EmbeddingModelPreset[];
}

type RawEmbeddingProviderPreset = Omit<EmbeddingProviderPreset, 'models'> & {
  models: LegacyCatalogModel[];
};

export const EMBEDDING_PROVIDER_PRESETS: EmbeddingProviderPreset[] =
  (embeddingProviderPresets as RawEmbeddingProviderPreset[]).map((preset) => ({
    ...preset,
    models: attachModelDescriptors(preset.models, {
      surface: 'embedding',
      providerId: canonicalModelProviderId(preset.id, preset.provider),
      endpointId: modelEndpointId('embedding', preset.id),
      region: inferModelCatalogRegion(preset.baseUrl),
      apiStyle: preset.apiStyle ?? 'openai_embeddings',
    }) as EmbeddingModelPreset[],
  }));

export function defaultEmbeddingModel(preset: EmbeddingProviderPreset): EmbeddingModelPreset | null {
  // Choosing a provider edits an unsaved configuration; it does not execute a
  // model or certify account access. Offer its documented recommendation while
  // retaining readiness badges and the explicit connection test.
  return selectImplicitDefault(preset.models) ?? preset.models.find(model => model.recommended
    && model.descriptor.lifecycle === 'active' && model.descriptor.access === 'public'
    && model.descriptor.availableToCredential !== false) ?? null;
}

export function findEmbeddingProviderPreset(baseUrl: string): EmbeddingProviderPreset | null {
  const normalized = normalizeModelEndpointUrl(baseUrl);
  return EMBEDDING_PROVIDER_PRESETS.find(
    (preset) => normalizeModelEndpointUrl(preset.baseUrl) === normalized || (preset.id === 'alibaba-model-studio-cn' && isModelStudioWorkspace(baseUrl)),
  ) ?? null;
}

function isModelStudioWorkspace(baseUrl: string): boolean {
  try {
    const url = new URL(baseUrl);
    return url.protocol === 'https:' && ['cn-beijing', 'ap-southeast-1', 'cn-hongkong'].some(region => url.hostname.endsWith(`.${region}.maas.aliyuncs.com`)) && url.pathname.replace(/\/$/, '') === '/compatible-mode/v1';
  } catch { return false; }
}

export function embeddingApiKeyRequired(baseUrl: string): boolean {
  try {
    const host = new URL(baseUrl).hostname;
    return host !== 'localhost' && host !== '[::1]' && !/^127(?:\.\d{1,3}){3}$/.test(host);
  } catch { return true; }
}
