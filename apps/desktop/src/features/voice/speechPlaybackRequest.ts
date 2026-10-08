import type { TextToSpeechConfig } from '../../types/conversation';
import { findTtsProviderPreset, ttsPresetEndpointMatches } from '../../lib/ttsProviderPresets';

export type SpeechPlaybackRequestResult<Preview> =
  | { kind: 'cancelled' }
  | { kind: 'input_limit'; model: string; actual: number; limit: number }
  | { kind: 'ready'; preview: Preview };

/** Enforce the selected endpoint's documented input limit. Unknown/private
 * models do not inherit an unrelated provider's or a generic text ceiling. */
export function speechPlaybackInputLimit(config: TextToSpeechConfig): number {
  const adapterLimit = config.apiStyle === 'dashscope_audio_generation' ? 3_000 : Number.POSITIVE_INFINITY;
  const preset = findTtsProviderPreset(config);
  if (!preset || !ttsPresetEndpointMatches(preset, config.baseUrl)) return adapterLimit;
  const model = preset.models.find(candidate => candidate.id === config.model.trim());
  return Math.min(adapterLimit, model?.maxInputCharacters ?? adapterLimit);
}

/** A single request, using one immutable configuration snapshot. Cancellation
 * during settings lookup must prevent synthesis, not merely hide its result. */
export async function requestSpeechPlayback<Preview>(
  text: string,
  dependencies: {
    loadConfig(): Promise<TextToSpeechConfig | undefined>;
    synthesize(text: string, config: TextToSpeechConfig): Promise<Preview>;
    isCurrent(): boolean;
  },
): Promise<SpeechPlaybackRequestResult<Preview>> {
  const loaded = await dependencies.loadConfig();
  if (!dependencies.isCurrent()) return { kind: 'cancelled' };
  if (!loaded) throw new Error('Text to speech is not configured.');
  const config = { ...loaded };
  const limit = speechPlaybackInputLimit(config);
  // Rust's chars().count() counts Unicode code points, not UTF-16 code units.
  const actual = Array.from(text).length;
  if (actual > limit) return { kind: 'input_limit', model: config.model, actual, limit };
  const preview = await dependencies.synthesize(text, config);
  return dependencies.isCurrent() ? { kind: 'ready', preview } : { kind: 'cancelled' };
}
