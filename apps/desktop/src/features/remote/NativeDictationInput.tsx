import { useEffect, useImperativeHandle, useRef, useState, type Ref } from 'react';
import { Mic, Square } from 'lucide-react';
import { useTranslation } from '../../i18n';
import type { NativeSpeech } from './remoteClient';
import type { DictationControls } from '../voice/useHoldToDictate';
import type { VoiceDictationEvent } from '../voice/voiceDraftProjection';
import { nativeSpeechErrorKey } from './nativeSpeechErrors';

export function NativeDictationInput({ speech, disabled, onEvent, onBusy, controls }: {
  speech: NativeSpeech; disabled: boolean; onEvent: (event: VoiceDictationEvent) => void; onBusy: (busy: boolean) => void;
  controls?: Ref<DictationControls>;
}) {
  const { t, locale } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [finishing, setFinishing] = useState(false);
  const [error, setError] = useState('');
  const active = useRef<{ id: string; finishing: boolean; prepared: boolean } | null>(null);
  const heldId = useRef<string | null>(null);
  const callbacks = useRef({ onEvent, onBusy }); callbacks.current = { onEvent, onBusy };
  const cancel = () => {
    const request = active.current;
    if (!request) return;
    active.current = null;
    callbacks.current.onEvent({ kind: 'cancel' }); callbacks.current.onBusy(false);
    setBusy(false); setFinishing(false);
    void speech.cancel(request.id).catch(() => {});
  };
  useEffect(() => () => {
    const request = active.current; active.current = null;
    if (request) { void speech.cancel(request.id).catch(() => {}); callbacks.current.onEvent({kind:'cancel'}); callbacks.current.onBusy(false); }
  }, [speech]);
  async function start() {
    if (disabled || active.current) return;
    const request = { id: crypto.randomUUID(), finishing: false, prepared: !speech.prepare };
    active.current = request; setBusy(true); setFinishing(false); onBusy(true); setError(''); onEvent({ kind: 'start' });
    try {
      if (speech.prepare) {
        await speech.prepare(request.id);
        if (active.current !== request) { await speech.cancel(request.id); return; }
        request.prepared = true;
      }
      const text = await speech.listen(locale, request.id);
      if (!text.trim()) throw new Error('speech_no_match');
      if (active.current === request) callbacks.current.onEvent({ kind:'final', text });
    } catch (cause) {
      if (active.current === request && !String(cause).includes('speech_cancelled')) setError(t(nativeSpeechErrorKey(cause)));
    } finally {
      if (active.current === request) { active.current = null; setBusy(false); setFinishing(false); callbacks.current.onBusy(false); callbacks.current.onEvent({ kind:'end' }); }
    }
  }
  async function finish() {
    const request = active.current;
    if (!request || request.finishing) return;
    if (!speech.finish || !request.prepared) { cancel(); return; }
    request.finishing = true; setFinishing(true);
    try { await speech.finish(request.id); }
    catch (cause) { if (active.current === request) { cancel(); setError(t(nativeSpeechErrorKey(cause))); } }
  }
  const readActiveId = () => active.current?.id ?? null;
  useImperativeHandle(controls, () => ({
    start: () => { if (!active.current) { void start(); heldId.current = readActiveId(); } },
    finish: () => { if (heldId.current && heldId.current === active.current?.id) void finish(); heldId.current = null; },
    cancel: () => { if (heldId.current && heldId.current === active.current?.id) cancel(); heldId.current = null; },
  }));
  return <div className="native-dictation"><button type="button" className="mobile-icon-button" disabled={disabled && !busy} aria-label={busy ? t(speech.finish && !finishing ? 'remote.stop' : 'common.cancel') : t('remote.voiceInput')} aria-pressed={busy} onClick={() => { if (!busy) void start(); else if (finishing) cancel(); else void finish(); }}>{busy ? <Square size={18}/> : <Mic size={18}/>}</button>{error && <p className="native-dictation-error" role="alert">{error}<button type="button" aria-label={t('common.close')} onClick={() => setError('')}>×</button></p>}</div>;
}
