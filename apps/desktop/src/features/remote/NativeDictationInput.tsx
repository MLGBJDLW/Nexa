import { useEffect, useRef, useState } from 'react';
import { Mic, Square } from 'lucide-react';
import { useTranslation } from '../../i18n';
import type { NativeSpeech } from './remoteClient';
import type { VoiceDictationEvent } from '../voice/voiceDraftProjection';

export function NativeDictationInput({ speech, disabled, onEvent, onBusy }: {
  speech: NativeSpeech; disabled: boolean; onEvent: (event: VoiceDictationEvent) => void; onBusy: (busy: boolean) => void;
}) {
  const { t, locale } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const generation = useRef(0);
  const active = useRef(false);
  const callbacks = useRef({ onEvent, onBusy }); callbacks.current = { onEvent, onBusy };
  useEffect(() => () => { generation.current++; if (active.current) { void speech.cancel().catch(() => {}); callbacks.current.onEvent({ kind: 'end' }); callbacks.current.onBusy(false); } }, [speech]);
  async function dictate() {
    if (busy) { generation.current++; active.current = false; setBusy(false); onBusy(false); onEvent({ kind:'cancel' }); await speech.cancel().catch(() => {}); return; }
    const request = ++generation.current;
    active.current = true; setBusy(true); onBusy(true); setError(''); onEvent({ kind: 'start' });
    try {
      const text = await speech.listen(locale);
      if (generation.current === request) callbacks.current.onEvent({ kind:'final', text });
    } catch (error) { if (generation.current === request) setError(String(error)); }
    finally { if (generation.current === request) { active.current = false; setBusy(false); callbacks.current.onBusy(false); callbacks.current.onEvent({ kind:'end' }); } }
  }
  return <div className="native-dictation"><button type="button" className="mobile-icon-button" disabled={disabled && !busy} aria-label={busy ? t('common.cancel') : t('remote.voiceInput')} aria-pressed={busy} onClick={() => void dictate()}>{busy ? <Square size={18}/> : <Mic size={18}/>}</button>{error && <p className="native-dictation-error" role="alert">{error}<button type="button" aria-label={t('common.close')} onClick={() => setError('')}>×</button></p>}</div>;
}
