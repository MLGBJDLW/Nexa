import { useId } from 'react';
import { useTranslation } from '../../desktop/src/i18n';
import { STT_PROVIDER_PRESETS, defaultSttItem } from '../../desktop/src/lib/sttProviderPresets';
import { TTS_PROVIDER_PRESETS, defaultTtsItem, ttsVoiceSupportsModel, ttsVoiceForModel } from '../../desktop/src/lib/ttsProviderPresets';
import type { SpeechToTextConfig, TextToSpeechConfig } from '../../desktop/src/types/conversation';

export const phoneInputPresets = STT_PROVIDER_PRESETS.filter(preset => !preset.local && ['openai_transcription', 'dashscope_asr', 'dashscope_audio_asr', 'gemini_interactions_asr', 'meta_asr'].includes(preset.apiStyle));
export const phoneOutputPresets = TTS_PROVIDER_PRESETS.filter(preset => !preset.local && ['openai_speech', 'elevenlabs_speech', 'mistral_speech', 'mistral_realtime_speech', 'minimax_speech', 'azure_speech', 'dashscope_speech', 'dashscope_audio_generation', 'gemini_interactions_speech'].includes(preset.apiStyle));
export function inputDefaults(): SpeechToTextConfig {
  const preset = phoneInputPresets[0];
  return { provider: preset.provider, apiStyle: preset.apiStyle, apiKey: '', baseUrl: preset.baseUrl, model: defaultSttItem(preset.models)?.id ?? '', language: null };
}
export function outputDefaults(): TextToSpeechConfig {
  const preset = phoneOutputPresets[0];
  const model = defaultTtsItem(preset.models)?.id ?? '';
  return { provider: preset.provider, apiStyle: preset.apiStyle, apiKey: '', baseUrl: preset.baseUrl, model, voice: ttsVoiceForModel(preset, model), outputFormat: preset.outputFormats.includes('mp3') ? 'mp3' : preset.outputFormats[0], speed: 1 };
}

export function PhoneCloudSpeechFields({ output = false, config, disabled, onChange }: {
  output?: boolean; config: SpeechToTextConfig | TextToSpeechConfig; disabled: boolean;
  onChange: (config: SpeechToTextConfig | TextToSpeechConfig) => void;
}) {
  const { t } = useTranslation(); const id = useId();
  const presets = output ? phoneOutputPresets : phoneInputPresets;
  const preset = presets.find(preset => preset.provider === config.provider && preset.apiStyle === config.apiStyle);
  const patch = (value: Partial<SpeechToTextConfig & TextToSpeechConfig>) => onChange({ ...config, ...value });
  const outputPreset = output ? phoneOutputPresets.find(item => item.id === preset?.id) : undefined;
  return <fieldset disabled={disabled} className="mobile-settings-form" data-testid={output ? 'phone-cloud-output' : 'phone-cloud-input'}>
    <label>{t('settings.provider')}<select value={preset?.id ?? ''} onChange={event => {
      const next = presets.find(item => item.id === event.target.value)!;
      const speech = output ? phoneOutputPresets.find(item => item.id === next.id) : undefined;
      const input = !output ? phoneInputPresets.find(item => item.id === next.id) : undefined;
      const model = (speech ? defaultTtsItem(speech.models)?.id : input ? defaultSttItem(input.models)?.id : '') ?? '';
      onChange({ ...config, provider: next.provider, apiStyle: next.apiStyle, baseUrl: next.baseUrl, apiKey: '', model,
        ...(speech ? { voice: defaultTtsItem(speech.voices.filter(voice => ttsVoiceSupportsModel(voice, model)))?.id ?? '', outputFormat: speech.outputFormats.includes('mp3') ? 'mp3' : speech.outputFormats[0], speed: 1 } : {}) });
    }}>{!preset && <option value="" disabled>{config.provider}</option>}{presets.map(preset => <option key={preset.id} value={preset.id}>{preset.name}</option>)}</select></label>
    <label>{t('remote.model')}<input list={`${id}-models`} value={config.model} autoCapitalize="none" spellCheck={false} onChange={event => {
      const model = event.target.value;
      patch({ model, ...(outputPreset && 'voice' in config ? { voice: ttsVoiceForModel(outputPreset, model, config.voice, config.baseUrl) } : {}) });
    }}/></label>
    <datalist id={`${id}-models`}>{preset?.models.map(model => <option key={model.id} value={model.id}>{model.name}</option>)}</datalist>
    <label>{t('settings.apiKey')}<input type="password" value={config.apiKey} autoComplete="off" onChange={event => patch({ apiKey: event.target.value })}/></label>
    <label>{t('settings.baseUrl')}<input type="url" value={config.baseUrl ?? ''} autoCapitalize="none" spellCheck={false} onChange={event => patch({ baseUrl: event.target.value })}/></label>
    {output && 'voice' in config && <label>{t('settings.ttsVoice')}<input list={`${id}-voices`} value={config.voice} onChange={event => patch({ voice: event.target.value })}/><datalist id={`${id}-voices`}>{outputPreset?.voices.filter(voice => ttsVoiceSupportsModel(voice, config.model)).map(voice => <option key={voice.id} value={voice.id}>{voice.name}</option>)}</datalist></label>}
  </fieldset>;
}
