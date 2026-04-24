import providerPresets from "../../../../shared/provider-presets.json";

export interface ProviderPreset {
  id: string;
  name: string;
  provider: string;
  baseUrl: string;
  models: {
    id: string;
    name: string;
    tagKey?: string;
    recommended?: boolean;
  }[];
  requiresApiKey: boolean;
  icon: string;
  description: string;
}

export const PROVIDER_PRESETS: ProviderPreset[] =
  providerPresets as ProviderPreset[];

function normalizePresetBaseUrl(baseUrl: string | null | undefined): string {
  return (baseUrl ?? "").trim().replace(/\/+$/, "").toLowerCase();
}

export function findProviderPreset(input: {
  provider: string;
  baseUrl?: string | null;
}): ProviderPreset | null {
  const provider = input.provider.trim();
  const normalizedBaseUrl = normalizePresetBaseUrl(input.baseUrl);

  if (normalizedBaseUrl) {
    const exactMatch = PROVIDER_PRESETS.find(
      (preset) =>
        preset.provider === provider &&
        normalizePresetBaseUrl(preset.baseUrl) === normalizedBaseUrl,
    );
    if (exactMatch) {
      return exactMatch;
    }
  }

  const providerMatches = PROVIDER_PRESETS.filter(
    (preset) => preset.provider === provider,
  );
  if (providerMatches.length === 1) {
    return providerMatches[0];
  }

  return null;
}
