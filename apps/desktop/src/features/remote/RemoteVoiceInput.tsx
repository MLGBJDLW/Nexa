import { useCallback, useEffect, useRef, useState } from 'react';
import { Loader2, Mic, Square, X } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { useVoiceRecorder } from '../voice/useVoiceRecorder';
import type { VoiceDictationEvent } from '../voice/voiceDraftProjection';
import { LiveAudioQueue } from '../live/liveAudioQueue';
import { encodeLiveAudio } from '../live/liveTransport';
import { RemoteNetworkError, type RemoteClient } from './remoteClient';
import { remoteButton } from './remoteUi';

export function RemoteVoiceInput({ client, disabled, onEvent, onBusy }: {
  client: RemoteClient;
  disabled: boolean;
  onEvent: (event: VoiceDictationEvent) => void;
  onBusy: (busy: boolean) => void;
}) {
  const { t } = useTranslation();
  const [phase, setPhase] = useState<'idle' | 'starting' | 'recording' | 'reconnecting' | 'finishing'>('idle');
  const [error, setError] = useState('');
  const recorder = useVoiceRecorder();
  const refs = useRef({ recorder, onEvent, onBusy, t }); refs.current = { recorder, onEvent, onBusy, t };
  const session = useRef<string | null>(null);
  const queue = useRef<LiveAudioQueue | null>(null);
  const mounted = useRef(true);
  const generation = useRef(0);
  const eventSequence = useRef(0);
  const finishPending = useRef(false);
  const ready = useRef(false);
  const connected = useRef(client.state.phase === 'connected');
  const transition = useRef(Promise.resolve());
  const reconnectDeadline = useRef<ReturnType<typeof setTimeout> | null>(null);
  const clear = useCallback((reason?: string) => {
    generation.current++;
    ready.current = false;
    if (reconnectDeadline.current) clearTimeout(reconnectDeadline.current);
    reconnectDeadline.current = null;
    const id = session.current; session.current = null;
    queue.current?.close(); queue.current = null;
    refs.current.recorder.cancelRecording();
    refs.current.onEvent({ kind:'end' }); refs.current.onBusy(false);
    if (mounted.current) { setPhase('idle'); if (reason) setError(reason); }
    if (id) void client.rpc('voice.cancel', { sessionId:id }).catch(() => {});
  }, [client]);
  useEffect(() => {
    mounted.current = true;
    const off = client.subscribe(event => {
      if (event.event !== 'voice:event' || event.payload.sessionId !== session.current) return;
      if (event.payload.sequence <= eventSequence.current) return;
      eventSequence.current = event.payload.sequence;
      if (event.payload.kind === 'interim' || event.payload.kind === 'final') {
        refs.current.onEvent({ kind:'interim', text:String(event.payload.text ?? '') });
      } else if (event.payload.kind === 'error') clear(String(event.payload.text || refs.current.t('remote.voiceFailed')));
    });
    const offConnection = client.subscribeConnection(state => {
      connected.current = state.phase === 'connected';
      const id = session.current;
      if (!id) return;
      if (state.phase === 'closed' || state.phase === 'revoked') { clear(refs.current.t('remote.voiceInterrupted')); return; }
      if (!connected.current) {
        queue.current?.pause();
        if (!finishPending.current) setPhase('reconnecting');
        if (!reconnectDeadline.current) reconnectDeadline.current = setTimeout(() => {
          if (session.current === id) clear(refs.current.t('remote.voiceInterrupted'));
        }, Math.min(30, client.paired.manifest.reconnectGraceSeconds) * 1000);
      }
      if (!ready.current || finishPending.current) return;
      const mine = generation.current;
      transition.current = transition.current.then(async () => {
        if (generation.current !== mine) return;
        if (!connected.current) { await refs.current.recorder.pauseRecording(); return; }
        const snapshot = await client.rpc<{ sessionId:string; sequence:number; text:string; phase:string; error?:string }>('voice.snapshot', { sessionId:id });
        if (generation.current !== mine || !connected.current) return;
        if (snapshot.error || snapshot.phase === 'failed') throw new Error(snapshot.error || refs.current.t('remote.voiceFailed'));
        if (snapshot.sequence >= eventSequence.current) {
          eventSequence.current = snapshot.sequence;
          refs.current.onEvent({ kind:'interim', text:snapshot.text });
        }
        if (snapshot.phase === 'finished') { clear(); return; }
        if (reconnectDeadline.current) clearTimeout(reconnectDeadline.current);
        reconnectDeadline.current = null;
        queue.current?.resume();
        await refs.current.recorder.resumeRecording();
        if (generation.current === mine) setPhase('recording');
      }).catch(error => {
        if (generation.current === mine && !(error instanceof RemoteNetworkError)) clear(error instanceof Error ? error.message : String(error));
      });
    });
    const leave = () => clear();
    window.addEventListener('pagehide', leave);
    return () => { mounted.current = false; off(); offConnection(); window.removeEventListener('pagehide', leave); clear(); };
  }, [client, clear]);
  async function start() {
    if (session.current || disabled) return;
    setError('');
    if (!window.isSecureContext || !navigator.mediaDevices) { setError(t('remote.microphoneNeedsHttps')); return; }
    const id = crypto.randomUUID();
    session.current = id; eventSequence.current = 0;
    const mine = ++generation.current;
    setPhase('starting'); onBusy(true); onEvent({ kind:'start' });
    try {
      const opened = await client.rpc<{ sessionId:string; sampleRate:number }>('voice.start', { requestId:id });
      if (generation.current !== mine) { void client.rpc('voice.cancel', { sessionId:id }).catch(() => {}); return; }
      const delivery = new LiveAudioQueue(Math.round(opened.sampleRate / 10) * 2,
        bytes => client.audio(id, encodeLiveAudio(bytes), 'voice.audio'),
        error => clear(error instanceof Error ? error.message : refs.current.t('remote.voiceInterrupted')), 16, () => client.recoverAudio());
      queue.current = delivery;
      if (!connected.current) delivery.pause();
      await refs.current.recorder.startRecording({ targetSampleRate:opened.sampleRate, onPcmChunk:bytes => delivery.append(bytes),
        onCaptureIssue:() => clear(refs.current.t('remote.voiceInterrupted')) });
      if (generation.current !== mine) { refs.current.recorder.cancelRecording(); return; }
      ready.current = true;
      if (!connected.current) { delivery.pause(); await refs.current.recorder.pauseRecording(); setPhase('reconnecting'); }
      else {
        if (reconnectDeadline.current) clearTimeout(reconnectDeadline.current);
        reconnectDeadline.current = null;
        setPhase('recording');
      }
    } catch (error) {
      if (generation.current !== mine) return;
      const detail = error instanceof Error ? error.message : String(error);
      clear(detail === 'remote_voice_setup_required' ? t('remote.voiceNeedsSetup') : error instanceof DOMException && error.name === 'NotAllowedError' ? t('remote.microphonePermission') : detail);
    }
  }
  async function finish() {
    const id = session.current;
    if (!id || finishPending.current) return;
    finishPending.current = true;
    const mine = generation.current;
    setPhase('finishing');
    try {
      await refs.current.recorder.stopRecording();
      await queue.current?.finish();
      if (generation.current !== mine) return;
      const result = await client.rpc<{ text:string }>('voice.finish', { sessionId:id });
      if (generation.current !== mine) return;
      onEvent({ kind:'final', text:result.text });
      clear();
    } catch (error) {
      if (generation.current === mine) clear(error instanceof Error ? error.message : String(error));
    } finally { finishPending.current = false; }
  }
  const busy = phase !== 'idle';
  return <div className="flex max-w-full flex-wrap items-center gap-2">
    <button type="button" className={`${remoteButton} ${phase === 'recording' ? 'text-danger' : ''}`} aria-label={phase === 'recording' ? t('remote.finishDictation') : t('remote.voiceInput')}
      disabled={(disabled && !busy) || phase === 'starting' || phase === 'finishing' || phase === 'reconnecting'} onClick={() => void (phase === 'recording' ? finish() : start())}>
      {phase === 'starting' || phase === 'finishing' || phase === 'reconnecting' ? <Loader2 size={16} className="animate-spin" /> : phase === 'recording' ? <Square size={16} /> : <Mic size={16} />}
      {phase === 'reconnecting' ? t('remote.reconnecting') : phase === 'recording' ? `${Math.floor(recorder.recordingDuration / 60)}:${String(recorder.recordingDuration % 60).padStart(2, '0')}` : phase === 'finishing' ? t('remote.voiceFinishing') : t('remote.voiceInput')}
    </button>
    {busy && <button type="button" className="p-2 text-text-secondary" aria-label={t('common.cancel')} onClick={() => clear()}><X size={16} /></button>}
    {phase === 'reconnecting' && <p role="status" className="w-full text-xs text-warning">{t('remote.voiceReconnecting')}</p>}
    {error && <p role="alert" className="w-full text-xs text-danger">{error}</p>}
  </div>;
}
