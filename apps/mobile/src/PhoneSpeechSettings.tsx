import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ChevronRight, Mic, Square } from 'lucide-react';
import { useTranslation } from '../../desktop/src/i18n';
import { nativeSpeechErrorKey } from '../../desktop/src/features/remote/nativeSpeechErrors';

interface SpeechState { mode: 'system' | 'on_device'; language: string; systemAvailable: boolean; onDeviceAvailable: boolean; microphoneGranted: boolean }

export function PhoneSpeechSettings({ open }: { open: boolean }) {
  const { t, availableLocales } = useTranslation();
  const [state, setState] = useState<SpeechState | null>(null);
  const [error, setError] = useState('');
  const [listening, setListening] = useState(false);
  const [sample, setSample] = useState('');
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    if (!open) return;
    let active = true;
    void invoke<SpeechState>('mobile_speech', { action: 'status' }).then(value => { if (active) setState(value); }).catch(error => { if (active) setError(String(error)); });
    return () => { active = false; void invoke('mobile_speech', { action: 'cancel' }).catch(() => {}); };
  }, [open]);
  const configure = async (patch: Partial<SpeechState>) => {
    if (!state || saving) return;
    setSaving(true); setError('');
    try { setState(await invoke<SpeechState>('mobile_speech', { action: 'configure', mode: state.mode, language: state.language, ...patch })); }
    catch (error) { setError(String(error)); } finally { setSaving(false); }
  };
  const test = async () => {
    if (listening) { await invoke('mobile_speech', { action: 'cancel' }); return; }
    setError(''); setSample(''); setListening(true);
    try { const result = await invoke<{ text: string }>('mobile_speech', { action: 'listen' }); setSample(result.text); }
    catch (error) { setError(t(nativeSpeechErrorKey(error))); }
    finally { setListening(false); }
  };
  const settings = (mode: string) => { void invoke('mobile_speech', { action: 'settings', mode }).catch(error => setError(String(error))); };
  const available = state?.mode === 'on_device' ? state.onDeviceAvailable : state?.systemAvailable;
  return <section className="mobile-settings-section" data-testid="phone-speech-settings">
    <h3>{t('remote.mobileSpeechSettings')}</h3>
    <p className="mobile-section-hint">{t('remote.mobileSpeechHint')}</p>
    {error && <p className="mobile-error" role="alert">{error}</p>}
    {state && <div className="mobile-settings-form">
      <label>{t('remote.mobileSpeechEngine')}<select aria-label={t('remote.mobileSpeechEngine')} disabled={saving || listening} value={state.mode} onChange={event => void configure({ mode: event.target.value as SpeechState['mode'] })}>
        <option value="system">{t('remote.mobileSpeechSystem')}</option><option value="on_device">{t('remote.mobileSpeechOnDevice')}</option>
      </select></label>
      <label>{t('remote.language')}<select aria-label={t('remote.mobileSpeechLanguage')} disabled={saving || listening} value={state.language} onChange={event => void configure({ language: event.target.value })}>
        <option value="">{t('settings.isDefault')}</option>{availableLocales.map(locale => <option key={locale.code} value={locale.code}>{locale.name}</option>)}
      </select></label>
      {!available && <p role="status" className="mobile-section-hint">{t(state.mode === 'on_device' ? 'remote.speechOnDeviceUnavailable' : 'remote.speechUnavailable')}</p>}
      <button type="button" disabled={!available || saving} onClick={() => void test()}>{listening ? <Square size={16}/> : <Mic size={16}/>}{t(listening ? 'common.cancel' : 'remote.mobileSpeechTest')}</button>
      {sample && <p role="status" className="mobile-section-hint">{sample}</p>}
    </div>}
    <button onClick={() => settings('system')}>{t('remote.mobileSpeechSystemSettings')}<ChevronRight size={17}/></button>
    <button onClick={() => settings('tts')}>{t('remote.mobileSpeechOutputSettings')}<ChevronRight size={17}/></button>
  </section>;
}
