import text from './provider-presets.json' with { type: 'json' };
import image from './image-provider-presets.json' with { type: 'json' };
import embedding from './embedding-provider-presets.json' with { type: 'json' };
import stt from './stt-provider-presets.json' with { type: 'json' };
import tts from './tts-provider-presets.json' with { type: 'json' };
import decisions from './system-one-provider-presets.json' with { type: 'json' };
import video from './video-provider-presets.json' with { type: 'json' };
import patterns from './dedicated-model-id-patterns.json' with { type: 'json' };

const dedicatedFamily = new RegExp(patterns.join('|'), 'i');
let dedicatedIds;

/** Providers with ID-only discovery need the operation catalogs to distinguish
 * speech, image and other dedicated models from conversational models. */
export function isChatModelId(value) {
  const id = value.trim().toLowerCase().replace(/^models\//, '');
  if (!id || dedicatedFamily.test(id)) return false;
  if (!dedicatedIds) {
    dedicatedIds = new Set();
    for (const preset of [...image, ...embedding, ...stt, ...tts, ...decisions, ...video]) {
      for (const model of preset.models) {
        for (const key of [model.id ?? model.modelId, ...(model.aliases ?? [])]) dedicatedIds.add(key.toLowerCase());
      }
    }
    // A model can independently implement both chat and a specialized API.
    for (const preset of text) {
      for (const model of preset.models) {
        for (const key of [model.id, ...(model.aliases ?? [])]) dedicatedIds.delete(key.toLowerCase());
      }
    }
  }
  return !dedicatedIds.has(id);
}
