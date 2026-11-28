import type { CapabilityPackageView } from '../types/conversation';
import {
  hydrateImageProviderPreset,
  isRuntimeImageProviderPreset,
  type RuntimeImageProviderPreset,
} from './imageProviderCatalogHydration.ts';
import type { ImageProviderPreset } from './imageProviderPresets';

export function extractImageProviderPresets(
  capabilityPackage: CapabilityPackageView | null,
  fallbackPresets: readonly ImageProviderPreset[],
): ImageProviderPreset[] {
  const catalog = capabilityPackage?.providerCatalogs?.find((item) => item.id === 'imageProviders');
  const presets = (catalog?.items ?? [])
    .filter(isRuntimeImageProviderPreset)
    .map(hydrateImageProviderPreset);
  return presets.length > 0 ? presets : [...fallbackPresets];
}

/** Keep exact retired rows authoritative through missing or stale discovery. */
export function mergeOpenRouterImageDiscovery(
  preset: ImageProviderPreset,
  discovered: RuntimeImageProviderPreset['models'],
  bundledPresets: readonly ImageProviderPreset[],
): ImageProviderPreset {
  if (preset.id !== 'openrouter-images' || preset.apiStyle !== 'openrouter_images') return preset;
  const known = bundledPresets.find(row =>
    row.id === preset.id && row.apiStyle === preset.apiStyle && row.baseUrl === preset.baseUrl);
  const retired = new Map(
    [...(known?.models ?? []), ...preset.models]
      .filter(row => row.descriptor.lifecycle === 'removed')
      .map(row => [row.id, row]),
  );
  return hydrateImageProviderPreset({
    ...preset,
    models: [...discovered.filter(row => !retired.has(row.id)), ...retired.values()],
  });
}
