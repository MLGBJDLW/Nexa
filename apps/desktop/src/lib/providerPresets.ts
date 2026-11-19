import providerPresets from "../../../../shared/provider-presets.json";
import { isChatModelId } from "../../../../shared/chat-model-ids.mjs";
export { isChatModelId };
import subscriptionPresets from "../../../../shared/subscription-runtime-presets.json";
import externalAgentPresets from "../../../../shared/external-agent-presets.json";
import retiredExternalAgents from "../../../../shared/retired-external-agents.json";
import { projectSnapshotModels, type ProviderModelCatalogSnapshot } from './providerModelCatalog';
import type { ProviderStreamingConfig } from '../types/conversation';
import type {
  ProviderCapabilities,
  ProviderModelPreset,
  ReasoningCapability,
} from './providerTypes';
import {
  alibabaWorkspaceRegion,
  attachModelDescriptors,
  canonicalModelProviderId,
  inferModelCatalogRegion,
  modelEndpointId,
  normalizeModelEndpointUrl,
  type LegacyCatalogModel,
  type NativeWebSearchCapability,
} from './modelCatalog';

export type {
  ModelCatalogSource,
  ModelLifecycleStatus,
  ProviderCapabilities,
  ProviderModelPreset,
  ReasoningCapability,
  ReasoningEffortLevel,
  ThinkingBudgetCapability,
} from './providerTypes';

export interface ProviderPreset {
  retired?: boolean;
  runtime?: 'copilot' | 'codex' | 'acp';
  command?: string;
  args?: string[];
  docsUrl?: string;
  registryId?: string;
  registryVersion?: string;
  distribution?: {
    npx?: { package: string; runnerArgs?: string[]; args?: string[]; env?: Record<string, string> };
    uvx?: { package: string; runnerArgs?: string[]; args?: string[]; env?: Record<string, string> };
    binary?: Record<string, { archive: string; cmd: string; args?: string[]; env?: Record<string, string>; sha256?: string }>;
  };
  id: string;
  name: string;
  provider: string;
  baseUrl: string;
  models: ProviderModelPreset[];
  requiresApiKey: boolean;
  apiStyle?: string;
  icon: string;
  description: string;
  capabilities?: ProviderCapabilities;
  nativeWebSearch?: NativeWebSearchCapability;
  streaming?: ProviderStreamingConfig;
}

type RawProviderPreset = Omit<ProviderPreset, 'models'> & { models: LegacyCatalogModel[] };

const apiPresets = providerPresets as RawProviderPreset[];
export const retiredProviderTypes: ReadonlySet<string> = new Set(
  apiPresets
    .filter(preset => preset.retired && !apiPresets.some(active => active.provider === preset.provider && !active.retired))
    .map(preset => preset.provider),
);

const retiredModels = new Map(apiPresets.map(preset => {
  const index = new Map<string, LegacyCatalogModel>();
  for (const model of preset.models.filter(model => model.status === 'removed')) {
    for (const id of [model.id, ...(model.aliases ?? [])]) index.set(normalizeModelId(id), model);
  }
  return [preset.id, index] as const;
}));

export function removedProviderModel(presetId: string, modelId: string): LegacyCatalogModel | undefined {
  const normalized = normalizeModelId(modelId);
  const id = presetId === 'google' ? normalized.replace(/^models\//, '') : normalized;
  return retiredModels.get(presetId)?.get(id);
}

export function isRemovedProviderModel(presetId: string, modelId: string): boolean {
  return Boolean(removedProviderModel(presetId, modelId));
}

export const PROVIDER_PRESETS: ProviderPreset[] = ([...providerPresets, ...subscriptionPresets, ...externalAgentPresets] as RawProviderPreset[]).map((preset) => ({
  ...preset,
  models: attachModelDescriptors(preset.models.filter(model => model.status !== 'removed'), {
    surface: 'text',
    providerId: canonicalModelProviderId(preset.id, preset.provider),
    endpointId: modelEndpointId('text', preset.id),
    region: inferModelCatalogRegion(preset.baseUrl),
    apiStyle: preset.apiStyle ?? (preset.provider === 'anthropic'
      ? 'anthropic_messages'
      : preset.provider === 'google'
        ? 'gemini_generate_content'
        : preset.provider === 'ollama'
          ? 'ollama_chat'
          : 'openai_chat'),
  }) as ProviderModelPreset[],
}));

/** The backend owns ACP versions and new directory entries. Keep the shared
 * fallback for offline/browser startup, then replace that slice atomically. */
export function applyExternalAgentPresets(presets: ProviderPreset[]): void {
  const retained = PROVIDER_PRESETS.filter(preset => preset.runtime !== 'acp');
  const supported = presets.filter(preset => !retiredExternalAgents.some(id => preset.registryId === id || preset.provider === `acp_${id.replace(/-/g, '_')}`));
  PROVIDER_PRESETS.splice(0, PROVIDER_PRESETS.length, ...retained, ...supported);
}

function normalizePresetBaseUrl(baseUrl: string | null | undefined): string {
  return normalizeModelEndpointUrl(baseUrl);
}

function providerKeyForPresetLookup(provider: string, normalizedBaseUrl: string): string {
  const isLegacyQwenPayg = provider === "qwen"
    && PROVIDER_PRESETS.some(
      (preset) => preset.provider === 'alibaba_model_studio'
        && normalizePresetBaseUrl(preset.baseUrl) === normalizedBaseUrl,
    );
  return isLegacyQwenPayg ? "alibaba_model_studio" : provider;
}

function cloudPresetId(provider: string, baseUrl: string): string | null {
  if (provider !== 'vertex' && provider !== 'bedrock') return null;
  try {
    const url = new URL(baseUrl);
    if (url.protocol !== 'https:' || url.port || url.username || url.password || url.search || url.hash) return null;
    const path = url.pathname.replace(/\/+$/, '');
    if (provider === 'bedrock') return /^bedrock-runtime\.[a-z0-9-]+\.amazonaws\.com(?:\.cn)?$/.test(url.hostname) && !path ? 'bedrock' : null;
    if (url.hostname !== 'aiplatform.googleapis.com' && !url.hostname.endsWith('-aiplatform.googleapis.com')) return null;
    const match = path.match(/^\/v1\/projects\/[^/]+\/locations\/[^/]+\/publishers\/(google|anthropic)$/);
    return match ? (match[1] === 'google' ? 'vertex' : 'vertex-anthropic') : null;
  } catch { return null; }
}

export function findProviderPreset(input: {
  provider: string;
  baseUrl?: string | null;
}): ProviderPreset | null {
  const provider = input.provider.trim();
  const normalizedBaseUrl = normalizePresetBaseUrl(input.baseUrl);
  const lookupProvider = providerKeyForPresetLookup(provider, normalizedBaseUrl);

  if (normalizedBaseUrl) {
    const exactMatch = PROVIDER_PRESETS.find(
      (preset) =>
        preset.provider === lookupProvider &&
        normalizePresetBaseUrl(preset.baseUrl) === normalizedBaseUrl,
    );
    if (exactMatch) {
      return exactMatch;
    }
    if (lookupProvider === 'moonshot' && normalizedBaseUrl === 'https://api.moonshot.cn/v1') {
      return PROVIDER_PRESETS.find(preset => preset.id === 'moonshot') ?? null;
    }
    if (lookupProvider === 'deep_seek' && normalizedBaseUrl === 'https://api.deepseek.com/v1') {
      return PROVIDER_PRESETS.find(preset => preset.id === 'deepseek') ?? null;
    }
    // Alibaba recommends workspace-dedicated PAYG hosts in production. They
    // share their regional model contract while credentials and cache identities
    // remain bound to the exact workspace URL elsewhere.
    const region = lookupProvider === 'alibaba_model_studio' ? alibabaWorkspaceRegion(input.baseUrl ?? '') : null;
    const workspacePresetId = region === 'cn-beijing' ? 'alibaba-model-studio'
      : region === 'ap-southeast-1' ? 'qwen-cloud-intl' : null;
    if (workspacePresetId) {
      return PROVIDER_PRESETS.find((preset) => preset.id === workspacePresetId) ?? null;
    }
    const cloudId = cloudPresetId(lookupProvider, normalizedBaseUrl);
    if (cloudId) return PROVIDER_PRESETS.find(preset => preset.id === cloudId) ?? null;
    // A configured endpoint is part of the capability identity. Familiar
    // provider labels or hosts must not make an edited endpoint inherit an
    // official model/reasoning contract.
    return null;
  }

  const providerMatches = PROVIDER_PRESETS.filter(
    (preset) => preset.provider === lookupProvider,
  );
  const defaultMatch = providerMatches.find(
    (preset) => preset.id === lookupProvider || preset.id.replace(/-/g, '_') === lookupProvider,
  );
  if (defaultMatch) {
    return defaultMatch;
  }
  if (providerMatches.length === 1) {
    return providerMatches[0];
  }

  return null;
}

function normalizeModelId(model: string | null | undefined): string {
  return (model ?? "").trim().toLowerCase();
}

export function findProviderModelPreset(input: {
  provider: string;
  baseUrl?: string | null;
  model?: string | null;
}): ProviderModelPreset | null {
  const preset = findProviderPreset(input);
  const model = normalizeModelId(input.model);
  if (!preset || !model) {
    return null;
  }
  return (
    preset.models.find((candidate) => [candidate.id, ...candidate.descriptor.aliases]
      .some((id) => normalizeModelId(id) === model)) ??
    null
  );
}

function hasCapabilities(capabilities: ProviderCapabilities): boolean {
  return Object.keys(capabilities).length > 0;
}

function mergeCapabilities(
  providerCapabilities: ProviderCapabilities | undefined,
  modelCapabilities: ProviderCapabilities | undefined,
): ProviderCapabilities | null {
  const merged: ProviderCapabilities = {};

  if (providerCapabilities) {
    Object.assign(merged, providerCapabilities);
  }
  if (modelCapabilities) {
    Object.assign(merged, modelCapabilities);
  }

  return hasCapabilities(merged) ? merged : null;
}

export function getProviderModelCapabilities(input: {
  provider: string;
  baseUrl?: string | null;
  model?: string | null;
}): ProviderCapabilities | null {
  const preset = findProviderPreset(input);
  if (!preset) {
    return null;
  }

  return mergeCapabilities(
    preset.capabilities,
    findProviderModelPreset(input)?.capabilities,
  );
}

export function getReasoningCapability(input: {
  provider: string;
  baseUrl?: string | null;
  model?: string | null;
}): ReasoningCapability | null {
  return getProviderModelCapabilities(input)?.reasoning ?? null;
}

/** A persisted discovery may predate the installed bundle. Preserve its account
 * evidence while exposing new bundled models with their original access status. */
export function resolveProviderModelCatalog(snapshot: ProviderModelCatalogSnapshot) {
  const observed = projectSnapshotModels(snapshot);
  const preset = findProviderPreset(snapshot);
  const bundled = new Map(preset?.models.map(model => [model.id.trim().toLowerCase(), model]));
  const tombstones = new Set<string>();
  const markRemoved = (id: string, aliases: readonly string[] = []) => {
    for (const name of [id, ...aliases]) tombstones.add(name.trim().toLowerCase());
  };
  for (const descriptor of [...(snapshot.tombstones ?? []), ...(snapshot.descriptors ?? []).filter(model => model.lifecycle === 'removed')]) {
    markRemoved(descriptor.id, descriptor.aliases);
  }
  for (const model of snapshot.models) {
    if (model.status === 'removed') markRemoved(model.id, model.aliases);
  }
  const isRemoved = (id: string) => tombstones.has(normalizeModelId(id))
    || Boolean(preset && isRemovedProviderModel(preset.id, id));
  // Keep the current release's picker order. An older account snapshot can
  // contribute access and additional IDs without burying newly shipped rows.
  const byId = new Map(bundled);
  for (const model of observed) {
    const id = normalizeModelId(model.id);
    const current = bundled.get(id);
    const verified = Date.parse(model.descriptor.lastVerifiedAt ?? '');
    // A previous release may have cached the model before its contract changed.
    // Account availability and inference receipts are independent of that contract.
    if (current && Date.parse(current.descriptor.lastVerifiedAt ?? '') > (Number.isFinite(verified) ? verified : Date.parse(snapshot.refreshedAt))) {
      byId.set(id, { ...current, descriptor: { ...current.descriptor,
        availableToCredential: model.descriptor.availableToCredential,
        productReadiness: model.descriptor.productReadiness === 'callable' && current.descriptor.productReadiness !== 'product_ready'
          ? 'callable' : current.descriptor.productReadiness,
      } });
    } else if (current && (current.tagKey !== model.tagKey || current.recommended !== model.recommended)) {
      byId.set(id, {
        ...model,
        tagKey: current.tagKey,
        recommended: current.recommended,
        descriptor: { ...model.descriptor, recommended: current.descriptor.recommended },
      });
    } else byId.set(id, model);
  }
  for (const model of [...(preset?.models ?? []), ...observed]) {
    if ([model.id, ...model.descriptor.aliases].some(isRemoved)) {
      markRemoved(model.id, model.descriptor.aliases);
    }
  }
  const isChat = (model: ProviderModelPreset) => isChatModelId(model.id)
    && model.descriptor.inputModalities.includes('text') && model.descriptor.outputModalities.includes('text');
  const canSuggest = (id: string) => {
    const model = byId.get(normalizeModelId(id));
    return model ? isChat(model) : isChatModelId(id);
  };
  return { models: [...byId.values()].filter(model => !isRemoved(model.id) && isChat(model)), isRemoved, canSuggest };
}

export function catalogModelsForSnapshot(snapshot: ProviderModelCatalogSnapshot): ProviderModelPreset[] {
  return resolveProviderModelCatalog(snapshot).models;
}
