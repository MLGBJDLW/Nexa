import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Check, ChevronRight, Loader2 } from 'lucide-react';
import { useTranslation } from '../../desktop/src/i18n';
import { PROVIDER_PRESETS } from '../../desktop/src/lib/providerPresets';
import { MobileDialog } from '../../desktop/src/features/remote/MobileChatShell';
import type { MobileState } from './MobileApp';
import type { NativeHost } from './nativeClient';

const presets = PROVIDER_PRESETS.filter(preset => !preset.runtime && preset.requiresApiKey && !['bedrock','vertex','github_copilot_api'].includes(preset.provider));
export default function PhoneSettingsSheet({ state, host, onClose, onChanged, onForgot }: { state: MobileState; host?: NativeHost; onClose: () => void; onChanged: () => Promise<void>; onForgot: () => void }) {
  const { t, locale, setLocale, availableLocales } = useTranslation();
  const initial = presets.find(preset => preset.provider === state.settings?.providerType && preset.baseUrl === state.settings.baseUrl)
    ?? presets.find(preset => preset.provider === state.settings?.providerType) ?? presets.find(preset => preset.id === 'openai')!;
  const [provider, setProvider] = useState(initial.id);
  const [model, setModel] = useState(state.settings?.model ?? initial.models[0]?.id ?? '');
  const [baseUrl, setBaseUrl] = useState(state.settings?.baseUrl ?? initial.baseUrl);
  const [apiKey, setApiKey] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const preset = presets.find(item => item.id === provider)!;
  const action = async (run: () => Promise<void>) => { setError(''); setBusy(true); try { await run(); await onChanged(); } catch (error) { setError(String(error)); } finally { setBusy(false); } };
  return <MobileDialog title={t('remote.mobileSettings')} onClose={onClose}>
    {error && <p role="alert" className="mobile-error">{error}</p>}
    <form className="mobile-settings-form" onSubmit={event => { event.preventDefault(); void action(async () => { await invoke('mobile_save_settings', { settings: { providerType: preset.provider, baseUrl: baseUrl.trim() || null, model: model.trim() }, apiKey }); setApiKey(''); onClose(); }); }}>
      <label>{t('settings.provider')}<select aria-label={t('settings.provider')} value={provider} onChange={event => { const next = presets.find(item => item.id === event.target.value)!; setProvider(next.id); setBaseUrl(next.baseUrl); setModel(next.models[0]?.id ?? ''); setApiKey(''); }}>{presets.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</select></label>
      <label>{t('remote.model')}<input aria-label={t('remote.model')} list="phone-models" value={model} onChange={event => setModel(event.target.value)} autoCapitalize="none" spellCheck={false}/></label><datalist id="phone-models">{preset.models.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</datalist>
      <label>{t('settings.apiKey')}<input aria-label={t('settings.apiKey')} type="password" value={apiKey} onChange={event => setApiKey(event.target.value)} placeholder={state.settings ? '••••••••' : ''} autoComplete="off"/></label>
      <details className="mobile-advanced" open={baseUrl !== preset.baseUrl || undefined}><summary>{t('remote.advancedOptions')}</summary><label>{t('settings.baseUrl')}<input aria-label={t('settings.baseUrl')} type="url" value={baseUrl} onChange={event => setBaseUrl(event.target.value)} autoCapitalize="none" spellCheck={false}/></label></details>
      <p className="mobile-section-hint">{t('remote.mobileCredentials')}</p><button className="mobile-primary" disabled={busy}>{busy && <Loader2 size={16} className="animate-spin"/>}{t('common.save')}</button>
    </form>
    <section className="mobile-settings-section"><h3>{t('remote.mobilePermissions')}</h3><p className="mobile-section-hint">{t('remote.mobileControlHint')}</p>
      <button onClick={() => void action(async () => { await invoke('mobile_permission', { permission: 'notifications' }); })}>{t('remote.mobileNotifications')}{state.capabilities.notifications ? <Check size={17}/> : <ChevronRight size={17}/>}</button>
      <button onClick={() => void action(async () => { await invoke('mobile_permission', { permission: 'accessibility' }); })}>{t('remote.mobileAccessibility')}{state.capabilities.accessibility ? <Check size={17}/> : <ChevronRight size={17}/>}</button>
    </section>
    <label className="mobile-language">{t('remote.language')}<select aria-label={t('remote.language')} value={locale} onChange={event => setLocale(event.target.value as typeof locale)}>{availableLocales.map(item => <option key={item.code} value={item.code}>{item.name}</option>)}</select></label>
    {host && <button className="mobile-forget" onClick={() => void action(async () => { await invoke('mobile_remote', { hostId: host.id, action: 'forget' }); onForgot(); onClose(); })}>{t('remote.forget')}</button>}
    <p className="mobile-version">Nexa Mobile {state.version}</p>
  </MobileDialog>;
}
