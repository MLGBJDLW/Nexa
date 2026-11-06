import credentialScopes from '../../../../shared/provider-credential-scopes.json';
import { normalizeModelEndpointUrl } from './modelCatalog';

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

const TRUSTED_CREDENTIAL_ENDPOINTS: Readonly<Record<string, string>> = credentialScopes.endpoints;

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
    && !endpoint.username
    && !endpoint.password
    && !endpoint.search
    && !endpoint.hash;
  const trustedScope = hasTrustedShape
    ? TRUSTED_CREDENTIAL_ENDPOINTS[normalizedBaseUrl]
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

export function findSharedProviderCredential<T extends ProviderCredentialConfig>(
  configs: T[],
  provider: string,
  baseUrl?: string | null,
): T | null {
  const targetScope = providerCredentialScope(provider, baseUrl);
  const candidates = configs.filter(
    (config) =>
      config.apiKey.trim().length > 0 &&
      providerCredentialScope(config.provider, config.baseUrl) === targetScope,
  );
  return candidates.find((config) => config.isDefault) ?? candidates[0] ?? null;
}
