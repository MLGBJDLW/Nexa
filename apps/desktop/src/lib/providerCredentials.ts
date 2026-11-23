import credentialScopes from '../../../../shared/provider-credential-scopes.json';
import { alibabaWorkspaceRegion, normalizeModelEndpointUrl } from './modelCatalog';

export interface ProviderCredentialConfig {
  provider: string;
  baseUrl?: string | null;
  apiKey: string;
  name: string;
  isDefault?: boolean;
}

function normalizedUrl(value: string | null | undefined): string {
  return normalizeModelEndpointUrl(value);
}

function endpointUrl(value: string | null | undefined): URL | null {
  const normalized = value?.trim();
  if (!normalized) return null;
  try {
    return new URL(normalized);
  } catch {
    return null;
  }
}

const WORKSPACE_ROUTES: Readonly<Record<string, Readonly<Record<string, string>>>> = credentialScopes.workspaceRoutes;
const TRUSTED_CREDENTIAL_ENDPOINTS: Readonly<Record<string, string>> = credentialScopes.endpoints;

function cleanCredentialUrl(url: URL): boolean {
  return !url.username && !url.password && !url.href.includes('?') && !url.href.includes('#');
}

function workspaceScope(url: URL): string | null {
  const region = alibabaWorkspaceRegion(new URL('/compatible-mode/v1', url.origin).href);
  if (!region || !Object.values(WORKSPACE_ROUTES[region] ?? {}).includes(url.pathname.replace(/\/+$/, ''))) return null;
  return `alibaba-workspace:${url.host}`;
}

/**
 * Resolve the credential boundary shared by chat and capability providers.
 *
 * Provider labels are not sufficient here: Alibaba chat uses
 * `alibaba_model_studio`, while its image/TTS catalogs historically use
 * `qwen`. Conversely, Token/Coding Plan keys are endpoint-scoped and must not
 * leak into pay-as-you-go DashScope services.
 */
export function providerCredentialScope(
  provider: string,
  baseUrl?: string | null,
): string {
  const normalizedProvider = provider.trim().toLowerCase();
  const normalizedBaseUrl = normalizedUrl(baseUrl);

  if (normalizedBaseUrl.includes('token-plan.') || normalizedBaseUrl.includes('coding.dashscope.')) {
    return `endpoint:${normalizedBaseUrl}`;
  }

  const endpoint = endpointUrl(baseUrl);
  const hasTrustedShape = endpoint != null && (endpoint.protocol === 'https:' || endpoint.protocol === 'wss:')
    && !endpoint.port
    && cleanCredentialUrl(endpoint);
  const trustedScope = hasTrustedShape
    ? TRUSTED_CREDENTIAL_ENDPOINTS[normalizedBaseUrl.replace(/\/$/, '')] ?? workspaceScope(endpoint)
    : undefined;
  if (trustedScope) return trustedScope;

  // Unknown or user-edited endpoints are never assumed to share credentials,
  // even when their provider label matches a known vendor.
  if (normalizedBaseUrl) {
    return `endpoint:${normalizedBaseUrl}`;
  }

  const providerAliases: Readonly<Record<string, string>> = credentialScopes.providerAliases;
  return providerAliases[normalizedProvider] ?? normalizedProvider;
}

type CredentialFeature = 'image' | 'video' | 'stt' | 'tts' | 'embedding' | 'systemOne' | 'webSearch';

interface CredentialFeaturePreset {
  id: string;
  baseUrl?: string | null;
  requiresApiKey?: boolean;
  apiStyle?: string;
  provider?: string;
  providerId?: string;
}

/** Return the reviewed destination of an implemented preset for this connection. */
export function credentialFeatureEndpoint(
  source: Pick<ProviderCredentialConfig, 'provider' | 'baseUrl'>,
  feature: CredentialFeature,
  preset: CredentialFeaturePreset,
): string | null {
  if (preset.requiresApiKey === false || preset.apiStyle === 'unverified') return null;
  const sourceUrl = endpointUrl(source.baseUrl);
  if (source.baseUrl?.trim() && (!sourceUrl || !cleanCredentialUrl(sourceUrl) || sourceUrl.protocol !== 'https:' || sourceUrl.port)) return null;
  const region = alibabaWorkspaceRegion(source.baseUrl ?? '');
  if (sourceUrl && region) {
    const path = WORKSPACE_ROUTES[region]?.[`${feature}:${preset.id}`];
    return path ? new URL(path, sourceUrl.origin).href : null;
  }
  const target = endpointUrl(preset.baseUrl);
  if (!target || !cleanCredentialUrl(target) || target.port) return null;
  const targetBase = normalizedUrl(target.href);
  const targetScope = TRUSTED_CREDENTIAL_ENDPOINTS[targetBase.replace(/\/$/, '')];
  if (!targetScope) return null;
  const sourceScope = providerCredentialScope(source.provider, source.baseUrl);
  if (sourceScope === targetScope) return preset.baseUrl ?? null;
  // The Singapore speech adapter uses the corresponding regional operations on
  // the Singapore authority. Individual model/voice availability is regional.
  if (sourceUrl && sourceScope === 'alibaba-model-studio:singapore' && targetScope === 'alibaba-model-studio:beijing'
    && (feature === 'stt' || feature === 'tts') && WORKSPACE_ROUTES['ap-southeast-1'][`${feature}:${preset.id}`]) {
    return new URL(target.pathname, sourceUrl.origin).href;
  }
  return null;
}

export function findSharedProviderCredential<T extends ProviderCredentialConfig>(
  configs: T[],
  provider: string,
  baseUrl?: string | null,
): T | null {
  const targetUrl = endpointUrl(baseUrl);
  if (baseUrl?.trim() && (!targetUrl || !cleanCredentialUrl(targetUrl))) return null;
  const targetScope = providerCredentialScope(provider, baseUrl);
  const candidates = configs.filter(config => {
    if (!config.apiKey.trim()) return false;
    if (config.baseUrl?.trim()) {
      const url = endpointUrl(config.baseUrl);
      if (!url || !cleanCredentialUrl(url)) return false;
    }
    return providerCredentialScope(config.provider, config.baseUrl) === targetScope;
  });
  return candidates.find((config) => config.isDefault) ?? candidates[0] ?? null;
}
