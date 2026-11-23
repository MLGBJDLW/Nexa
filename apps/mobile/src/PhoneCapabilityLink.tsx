import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { CapabilityLinkDialog, type CapabilityLinkOption } from '../../desktop/src/components/settings/CapabilityLinkDialog';
import { MobileDialog } from '../../desktop/src/features/remote/MobileChatShell';
import { useTranslation } from '../../desktop/src/i18n';
import { findProviderPreset } from '../../desktop/src/lib/providerPresets';
import type { WebSearchExecutionMode, WebSearchProviderMode } from '../../desktop/src/types/conversation';
import type { PhoneConnection } from './MobileApp';
import { phoneInputPresets, phoneOutputPresets, selectPhoneSpeechRoute,
  type PhoneSpeechConnection, type PhoneSpeechInput, type PhoneSpeechOutput } from './PhoneCloudSpeechFields';

interface Capabilities {
  id: string; name: string; provider: string; model: string; baseUrl: string | null;
  speech: PhoneSpeechConnection | null; speechToText: PhoneSpeechInput; textToSpeech: PhoneSpeechOutput;
  mode: 'system' | 'on_device' | 'cloud' | null; outputMode: 'system' | 'cloud' | null;
  nativeSearch: boolean; nativeSearchActive: boolean; deepseek: boolean; webConnectionId: string | null;
  webConfigured: boolean; webMode: WebSearchExecutionMode; webProviderMode: WebSearchProviderMode;
}

const read = (id: string) => invoke<Capabilities>('mobile_connection', { action: 'capabilities', id });

export function PhoneCapabilityLink({ id, name, connections, onDismiss }: {
  id: string; name: string; connections: PhoneConnection[]; onDismiss: () => void;
}) {
  const { t } = useTranslation();
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [error, setError] = useState('');
  const [attempt, retry] = useState(0);
  useEffect(() => {
    let active = true;
    setError('');
    void read(id).then(result => { if (active) setCaps(result); })
      .catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; };
  }, [id, attempt]);
  if (!caps) return <MobileDialog title={t('providers.capabilityTitle', { name })} onClose={onDismiss}>
    <p role={error ? 'alert' : 'status'}>{error || t('common.loading')}</p>
    <div className="mobile-confirm-actions">
      <button type="button" onClick={onDismiss}>{t('providers.capabilitySkip')}</button>
      {error && <button type="button" className="mobile-primary" onClick={() => retry(value => value + 1)}>{t('common.retry')}</button>}
    </div>
  </MobileDialog>;

  const accountName = (accountId?: string | null) => connections.find(c => c.id === accountId)?.name;
  const options: CapabilityLinkOption[] = [{ id: 'chat', label: t('remote.chat'), detail: caps.model, linked: true }];
  const model = findProviderPreset(caps)?.models.find(model => model.id === caps.model || model.descriptor.aliases.includes(caps.model));
  if (model?.descriptor.capabilities.vision || model?.descriptor.inputModalities.includes('image')) {
    options.push({ id: 'vision', label: t('providers.tagVision'), detail: t('chat.attachImage'), linked: true });
  }
  if (model?.descriptor.capabilities.reasoning) {
    options.push({ id: 'reasoning', label: t('chat.contextReasoningLabel'), detail: t('providers.capabilityAvailable'), linked: true });
  }
  if (model?.descriptor.capabilities.toolCalling) {
    options.push({ id: 'tools', label: t('chat.contextSegmentTools'), detail: t('providers.capabilityAvailable'), linked: true });
  }
  if (caps.nativeSearch) options.push({
    id: 'native', label: t('settings.webSearchNativeSupport'), linked: caps.nativeSearchActive, group: 'web',
    detail: caps.nativeSearchActive ? t('providers.capabilityAvailable') : t('providers.capabilityReplaces', { name: t('settings.webSearchExecutionMode.nexaRouter') }),
  });
  if (caps.deepseek) {
    const routing = t('settings.webSearchExecutionMode.nexaRouter') + ' · ' + t('settings.webSearchProviderMode.custom_first');
    const previous = caps.webConfigured && (accountName(caps.webConnectionId) ?? t('settings.webSearchCredentials'));
    options.push({ id: 'deepseek', group: 'web', label: t('settings.webSearchCustomProvider.deepseek'),
      linked: caps.webConfigured && caps.webConnectionId === id && caps.webMode === 'nexaRouter' && caps.webProviderMode === 'custom_first',
      detail: previous ? routing + ' · ' + t('providers.capabilityReplaces', { name: previous }) : routing });
  }
  const selectedSpeech: Record<string, PhoneSpeechInput | PhoneSpeechOutput> = {};
  for (const direction of ['input', 'output'] as const) {
    const input = direction === 'input';
    const current = input ? caps.speechToText : caps.textToSpeech;
    const mode = input ? caps.mode : caps.outputMode;
    const presets = input ? phoneInputPresets : phoneOutputPresets;
    for (const route of caps.speech?.[direction] ?? []) {
      const preset = presets.find(preset => preset.id === route.presetId);
      if (!preset) continue;
      const optionId = direction + '/' + preset.id;
      const replacement = accountName(current.connectionId) ?? current.model;
      const linked = mode === 'cloud' && current.connectionId === id && current.baseUrl === route.baseUrl &&
        current.provider === preset.provider && current.apiStyle === preset.apiStyle;
      const next = selectPhoneSpeechRoute(current, preset, route.baseUrl, id);
      selectedSpeech[optionId] = next;
      const prior = mode === 'cloud' ? replacement : t(mode === 'on_device' ? 'remote.mobileSpeechOnDevice' : 'remote.mobileSpeechSystem');
      const detail = [next.model, route.baseUrl, linked ? t('providers.capabilityLinked') : t('providers.capabilityReplaces', { name: prior })];
      options.push({ id: optionId, group: direction, linked, label: t(input ? 'settings.speechToText' : 'settings.textToSpeech') + ' · ' + preset.name, detail: detail.join(' · ') });
    }
  }
  async function apply(ids: string[]) {
    const args: Record<string, unknown> = { action: 'configure' };
    for (const selected of ids) {
      const speech = selectedSpeech[selected];
      if (!speech) continue;
      if (selected.startsWith('input/')) { args.mode = 'cloud'; args.speechToText = speech; }
      else { args.outputMode = 'cloud'; args.textToSpeech = speech; }
    }
    if (args.mode || args.outputMode) await invoke('mobile_speech', args);
    const web = ids.find(selected => selected === 'deepseek' || selected === 'native');
    if (web) await invoke('mobile_web_settings', { action: web === 'deepseek' ? 'use_deepseek' : 'use_native', connectionId: id });
  }
  return <CapabilityLinkDialog providerName={caps.name} options={options} onApply={apply} onDismiss={onDismiss}/>;
}
