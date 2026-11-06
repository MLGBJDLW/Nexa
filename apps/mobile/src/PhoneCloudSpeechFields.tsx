import { useId } from 'react';
import { useTranslation } from '../../desktop/src/i18n';
import { STT_PROVIDER_PRESETS, defaultSttItem, type SttProviderPreset } from '../../desktop/src/lib/sttProviderPresets';
import { TTS_PROVIDER_PRESETS, defaultTtsItem, ttsVoiceSupportsModel, ttsVoiceForModel, type TtsProviderPreset } from '../../desktop/src/lib/ttsProviderPresets';
import type { SpeechToTextConfig, TextToSpeechConfig } from '../../desktop/src/types/conversation';

export const phoneInputPresets = STT_PROVIDER_PRESETS.filter(preset => !preset.local && ['openai_transcription', 'dashscope_asr', 'dashscope_audio_asr', 'gemini_interactions_asr', 'meta_asr'].includes(preset.apiStyle));
export const phoneOutputPresets = TTS_PROVIDER_PRESETS.filter(preset => !preset.local && ['openai_speech', 'elevenlabs_speech', 'mistral_speech', 'mistral_realtime_speech', 'minimax_speech', 'azure_speech', 'dashscope_speech', 'dashscope_audio_generation', 'gemini_interactions_speech'].includes(preset.apiStyle));
interface SpeechRoute { presetId: string; baseUrl: string }
export interface PhoneSpeechConnection {
  id: string; name: string; input: SpeechRoute[]; output: SpeechRoute[];
}
export type PhoneSpeechInput = SpeechToTextConfig & { connectionId?: string | null };
export type PhoneSpeechOutput = TextToSpeechConfig & { connectionId?: string | null };

export function inputDefaults(preset = phoneInputPresets[0]): PhoneSpeechInput {
  return { provider: preset.provider, apiStyle: preset.apiStyle, apiKey: '', baseUrl: preset.baseUrl, model: defaultSttItem(preset.models)?.id ?? '', language: null };
}
export function outputDefaults(preset = phoneOutputPresets[0]): PhoneSpeechOutput {
  const model = defaultTtsItem(preset.models)?.id ?? '';
  return { provider: preset.provider, apiStyle: preset.apiStyle, apiKey: '', baseUrl: preset.baseUrl, model, voice: ttsVoiceForModel(preset, model), outputFormat: preset.outputFormats.includes('mp3') ? 'mp3' : preset.outputFormats[0], speed: 1 };
}

type PhoneSpeechConfig = PhoneSpeechInput | PhoneSpeechOutput;
function selectPreset(config: PhoneSpeechConfig, preset: SttProviderPreset | TtsProviderPreset, baseUrl: string, connectionId?: string): PhoneSpeechConfig {
  const sameService = config.provider === preset.provider && config.apiStyle === preset.apiStyle;
  let next = config;
  if (!sameService) {
    next = 'voices' in preset ? { ...config, ...outputDefaults(preset) } : {
      ...config, ...inputDefaults(preset), language: 'language' in config ? config.language : null,
    };
  }
  return { ...next, baseUrl, connectionId, apiKey: '' };
}

export function bindSpeechConnection(config: PhoneSpeechConfig, connection: PhoneSpeechConnection): PhoneSpeechConfig {
  const output = 'voice' in config;
  const routes = output ? connection.output : connection.input;
  const presets = output ? phoneOutputPresets : phoneInputPresets;
  const previous = presets.find(item => item.provider === config.provider && item.apiStyle === config.apiStyle);
  const route = routes.find(item => item.presetId === previous?.id) ?? routes[0];
  const preset = presets.find(item => item.id === route?.presetId);
  return preset ? selectPreset(config, preset, route.baseUrl, connection.id) : config;
}

export function PhoneCloudSpeechFields({ output = false, config, connections, disabled, onChange }: {
  output?: boolean; config: PhoneSpeechConfig; connections: PhoneSpeechConnection[]; disabled: boolean;
  onChange: (config: PhoneSpeechConfig) => void;
}) {
  const { t } = useTranslation(); const id = useId();
  const presets = output ? phoneOutputPresets : phoneInputPresets;
  const preset = presets.find(preset => preset.provider === config.provider && preset.apiStyle === config.apiStyle);
  const direction = output ? 'output' : 'input';
  const accounts = connections.filter(account => account[direction].length > 0);
  const selected = accounts.find(account => account.id === config.connectionId);
  const routes = selected?.[direction];
  const choices = config.connectionId ? presets.filter(item => routes?.some(route => route.presetId === item.id)) : presets;
  const available = !config.connectionId || routes?.some(route => route.presetId === preset?.id && route.baseUrl === config.baseUrl);
  const patch = (value: Partial<SpeechToTextConfig & TextToSpeechConfig>) => onChange({ ...config, ...value });
  const outputPreset = output ? phoneOutputPresets.find(item => item.id === preset?.id) : undefined;
  return <fieldset disabled={disabled} className="mobile-settings-form" data-testid={output ? 'phone-cloud-output' : 'phone-cloud-input'}>
    {(accounts.length > 0 || config.connectionId) && <label>{t('remote.model')}<select aria-label={t('remote.model')} value={config.connectionId ?? ''} onChange={event => {
      const account = accounts.find(item => item.id === event.target.value);
      onChange(account ? bindSpeechConnection(config, account) : { ...config, connectionId: null, apiKey: '' });
    }}>
      {config.connectionId && !selected && <option disabled value={config.connectionId}>{t('settings.shellUnavailable')}</option>}
      {accounts.map(account => <option key={account.id} value={account.id}>{account.name}</option>)}
      <option value="">{t('settings.customProvider')}</option>
    </select></label>}
    <label>{t('settings.provider')}<select aria-label={t('settings.provider')} value={available ? preset?.id ?? '' : ''} onChange={event => {
      const next = choices.find(item => item.id === event.target.value);
      if (next) onChange(selectPreset(config, next, routes?.find(route => route.presetId === next.id)?.baseUrl ?? next.baseUrl, selected?.id));
    }}>{(!preset || !available) && <option value="" disabled>{available ? config.provider : t('settings.shellUnavailable')}</option>}{choices.map(preset => <option key={preset.id} value={preset.id}>{preset.name}</option>)}</select></label>
    <label>{t('settings.model')}<input list={`${id}-models`} value={config.model} autoCapitalize="none" spellCheck={false} onChange={event => {
      const model = event.target.value;
      patch({ model, ...(outputPreset && 'voice' in config ? { voice: ttsVoiceForModel(outputPreset, model, config.voice, config.baseUrl) } : {}) });
    }}/></label>
    <datalist id={`${id}-models`}>{preset?.models.map(model => <option key={model.id} value={model.id}>{model.name}</option>)}</datalist>
    {!config.connectionId && <>
      <label>{t('settings.apiKey')}<input type="password" value={config.apiKey} autoComplete="off" onChange={event => patch({ apiKey: event.target.value })}/></label>
      <label>{t('settings.baseUrl')}<input type="url" value={config.baseUrl ?? ''} autoCapitalize="none" spellCheck={false} onChange={event => patch({ baseUrl: event.target.value })}/></label>
    </>}
    {output && 'voice' in config && <label>{t('settings.ttsVoice')}<input list={`${id}-voices`} value={config.voice} onChange={event => patch({ voice: event.target.value })}/><datalist id={`${id}-voices`}>{outputPreset?.voices.filter(voice => ttsVoiceSupportsModel(voice, config.model)).map(voice => <option key={voice.id} value={voice.id}>{voice.name}</option>)}</datalist></label>}
  </fieldset>;
}
