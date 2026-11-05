import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ChevronRight, Mic, Square, Volume2 } from 'lucide-react';
import { useTranslation } from '../../desktop/src/i18n';
import { nativeSpeechErrorKey } from '../../desktop/src/features/remote/nativeSpeechErrors';
import type { SpeechToTextConfig, TextToSpeechConfig } from '../../desktop/src/types/conversation';
import { PhoneCloudSpeechFields, inputDefaults, outputDefaults, phoneInputPresets, phoneOutputPresets } from './PhoneCloudSpeechFields';

interface SpeechState {
  mode: 'system' | 'on_device' | 'cloud'; outputMode: 'system' | 'cloud'; language: string;
  systemAvailable: boolean; onDeviceAvailable: boolean; microphoneGranted: boolean;
  speechToText: SpeechToTextConfig; textToSpeech: TextToSpeechConfig;
}
function editable(value: SpeechState): SpeechState {
  return { ...value, outputMode: value.outputMode ?? 'system',
    speechToText: phoneInputPresets.some(preset => preset.apiStyle === value.speechToText?.apiStyle) ? value.speechToText : inputDefaults(),
    textToSpeech: phoneOutputPresets.some(preset => preset.apiStyle === value.textToSpeech?.apiStyle) ? value.textToSpeech : outputDefaults() };
}

export function PhoneSpeechSettings({ open }: { open: boolean }) {
  const { t, availableLocales } = useTranslation();
  const [state, setState] = useState<SpeechState | null>(null);
  const [error, setError] = useState('');
  const [listening, setListening] = useState(false);
  const [finishing, setFinishing] = useState(false);
  const [speaking, setSpeaking] = useState(false);
  const [sample, setSample] = useState('');
  const [saving, setSaving] = useState(false);
  const generation = useRef(0);
  const dirty = useRef(false);
  useEffect(() => {
    if (!open) return;
    let active = true;
    const refresh = () => { void invoke<SpeechState>('mobile_speech', { action: 'status' }).then(value => { if (active) setState(current => dirty.current && current ? { ...current, systemAvailable: value.systemAvailable, onDeviceAvailable: value.onDeviceAvailable, microphoneGranted: value.microphoneGranted } : editable(value)); }).catch(error => { if (active) setError(t(nativeSpeechErrorKey(error))); }); };
    refresh(); window.addEventListener('focus', refresh);
    return () => { active = false; generation.current++; window.removeEventListener('focus', refresh); void invoke('mobile_speech', { action: 'cancel' }).catch(() => {}); void invoke('mobile_speech', { action: 'stop' }).catch(() => {}); };
  }, [open, t]);
  const change = (patch: Partial<SpeechState>) => { dirty.current = true; setState(current => current ? { ...current, ...patch } : current); };
  const save = async () => {
    if (!state || saving) return false;
    setSaving(true); setError('');
    try {
      const saved = await invoke<SpeechState>('mobile_speech', { action: 'configure', mode: state.mode, outputMode: state.outputMode, language: state.language,
        speechToText: state.mode === 'cloud' ? state.speechToText : undefined, textToSpeech: state.outputMode === 'cloud' ? state.textToSpeech : undefined });
      dirty.current = false; setState(editable(saved)); return true;
    } catch (error) { setError(t(nativeSpeechErrorKey(error))); return false; } finally { setSaving(false); }
  };
  const test = async () => {
    if (listening) {
      if (!finishing) { setFinishing(true); await invoke('mobile_speech', { action: 'finish' }).catch(error => setError(t(nativeSpeechErrorKey(error)))); }
      else { generation.current++; setListening(false); await invoke('mobile_speech', { action: 'cancel' }).catch(() => {}); }
      return;
    }
    if (!(await save())) return;
    const request = ++generation.current;
    setError(''); setSample(''); setListening(true); setFinishing(false);
    try { const result = await invoke<{ text: string }>('mobile_speech', { action: 'listen', language: state?.language }); if (!result.text.trim()) throw new Error('speech_no_match'); if (generation.current === request) setSample(result.text); }
    catch (error) { if (generation.current === request && !String(error).includes('speech_cancelled')) setError(t(nativeSpeechErrorKey(error))); }
    finally { if (generation.current === request) setListening(false); }
  };
  const readSample = async () => {
    if (speaking) { generation.current++; setSpeaking(false); await invoke('mobile_speech', { action: 'stop' }).catch(() => {}); return; }
    if (!(await save())) return;
    const request = ++generation.current; setSpeaking(true);
    try { await invoke('mobile_speech', { action: 'speak', text: sample || t('remote.mobileSpeechSample'), language: state?.language }); }
    catch (error) { if (generation.current === request) setError(t(nativeSpeechErrorKey(error))); }
    finally { if (generation.current === request) setSpeaking(false); }
  };
  const settings = (mode: string) => { void invoke('mobile_speech', { action: 'settings', mode }).catch(error => setError(String(error))); };
  const available = state?.mode === 'cloud' || (state?.mode === 'on_device' ? state.onDeviceAvailable : state?.systemAvailable || state?.onDeviceAvailable);
  const busy = saving || listening || speaking;
  return <section className="mobile-settings-section" data-testid="phone-speech-settings">
    <h3>{t('remote.mobileSpeechSettings')}</h3>
    <p className="mobile-section-hint">{t('remote.mobileSpeechChoiceHint')}</p>
    {error && <p className="mobile-error" role="alert">{error}</p>}
    {state && <div className="mobile-settings-form">
      <label>{t('remote.mobileSpeechEngine')}<select disabled={busy} value={state.mode} onChange={event => change({ mode: event.target.value as SpeechState['mode'] })}>
        <option value="system">{t('remote.mobileSpeechSystem')}</option><option value="on_device">{t('remote.mobileSpeechOnDevice')}</option><option value="cloud">{t('remote.mobileSpeechCloud')}</option>
      </select></label>
      {state.mode === 'cloud' && <><PhoneCloudSpeechFields config={state.speechToText} disabled={busy} onChange={value => change({ speechToText: value as SpeechToTextConfig })}/><p className="mobile-section-hint">{t('remote.mobileSpeechRecordHint')}</p></>}
      <label>{t('remote.mobileSpeechOutput')}<select aria-label={t('remote.mobileSpeechOutput')} disabled={busy} value={state.outputMode} onChange={event => change({ outputMode: event.target.value as SpeechState['outputMode'] })}><option value="system">{t('remote.mobileSpeechSystem')}</option><option value="cloud">{t('remote.mobileSpeechCloud')}</option></select></label>
      {state.outputMode === 'cloud' && <PhoneCloudSpeechFields output config={state.textToSpeech} disabled={busy} onChange={value => change({ textToSpeech: value as TextToSpeechConfig })}/>}
      <label>{t('remote.language')}<select aria-label={t('remote.mobileSpeechLanguage')} disabled={busy} value={state.language} onChange={event => change({ language: event.target.value })}>
        <option value="">{t('settings.isDefault')}</option>{availableLocales.map(locale => <option key={locale.code} value={locale.code}>{locale.name}</option>)}
      </select></label>
      <button type="button" disabled={busy} onClick={() => void save()}>{t('common.save')}</button>
      {!available && <p role="status" className="mobile-section-hint">{t(state.mode === 'on_device' ? 'remote.speechOnDeviceUnavailable' : 'remote.speechUnavailable')}</p>}
      <button type="button" disabled={!available || saving || speaking} onClick={() => void test()}>{listening ? <Square size={16}/> : <Mic size={16}/>}{t(listening ? finishing ? 'common.cancel' : 'remote.stop' : 'remote.mobileSpeechTest')}</button>
      <button type="button" disabled={saving || listening} onClick={() => void readSample()}>{speaking ? <Square size={16}/> : <Volume2 size={16}/>}{t(speaking ? 'remote.stopReading' : 'remote.readAloud')}</button>
      {sample && <p role="status" className="mobile-section-hint">{sample}</p>}
    </div>}
    <button onClick={() => settings('system')}>{t('remote.mobileSpeechSystemSettings')}<ChevronRight size={17}/></button>
    <button onClick={() => settings('tts')}>{t('remote.mobileSpeechOutputSettings')}<ChevronRight size={17}/></button>
  </section>;
}
