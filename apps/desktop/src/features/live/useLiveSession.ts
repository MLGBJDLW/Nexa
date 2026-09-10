import { useCallback, useEffect, useRef, useState } from 'react';
import { useVoiceRecorder } from '../voice/useVoiceRecorder';
import { LiveAudioQueue } from './liveAudioQueue';
import { applyLiveEvent, liveEnded, type LiveSnapshot, type LiveTransport, type StartLiveRequest } from './liveTransport';

export type LiveVideoSource = 'none' | 'camera' | 'screen';
const message = (error: unknown) => error instanceof Error ? error.message : String(error);

async function captureFrame(video: HTMLVideoElement): Promise<string | null> {
  if (!video.videoWidth || video.readyState < 2) return null;
  const canvas = document.createElement('canvas');
  const scale = Math.min(1, 960 / Math.max(video.videoWidth, video.videoHeight));
  canvas.width = Math.round(video.videoWidth * scale); canvas.height = Math.round(video.videoHeight * scale);
  canvas.getContext('2d')?.drawImage(video, 0, 0, canvas.width, canvas.height);
  for (const quality of [0.65, 0.4, 0.2]) {
    const blob = await new Promise<Blob | null>(resolve => canvas.toBlob(resolve, 'image/jpeg', quality));
    if (!blob || blob.size > 190 * 1024) continue;
    return new Promise<string>((resolve, reject) => {
      const reader = new FileReader(); reader.onerror = () => reject(new Error('Unable to encode Live frame'));
      reader.onload = () => resolve(String(reader.result).split(',')[1]); reader.readAsDataURL(blob);
    });
  }
  throw new Error('Live frame is too large. Use a smaller capture area.');
}

export function useLiveSession(transport: LiveTransport) {
  const [snapshot, setSnapshot] = useState<LiveSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [preview, setPreview] = useState<MediaStream | null>(null);
  const [reconnecting, setReconnecting] = useState(false);
  const connected = useRef(true);
  const captureTransition = useRef<Promise<void>>(Promise.resolve());
  const reconnectDeadline = useRef<ReturnType<typeof setTimeout> | null>(null);
  const voice = useVoiceRecorder();
  const voiceRef = useRef(voice); voiceRef.current = voice;
  const active = useRef<string | null>(null);
  const starting = useRef(false);
  const generation = useRef(0);
  const media = useRef<MediaStream | null>(null);
  const video = useRef<HTMLVideoElement | null>(null);
  const audioQueue = useRef<LiveAudioQueue | null>(null);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);
  const mounted = useRef(true);
  const refreshing = useRef(false);
  const clearCapture = useCallback(() => {
    if (reconnectDeadline.current) clearTimeout(reconnectDeadline.current); reconnectDeadline.current = null;
    voiceRef.current.cancelRecording(); audioQueue.current?.close(); audioQueue.current = null;
    if (timer.current) clearInterval(timer.current); timer.current = null;
    media.current?.getTracks().forEach(track => { track.onended = null; track.stop(); }); media.current = null;
    if (video.current) { video.current.srcObject = null; video.current = null; }
    if (mounted.current) setPreview(null);
  }, []);
  const stop = useCallback(async () => {
    generation.current++; clearCapture();
    starting.current = false;
    const id = active.current; active.current = null;
    if (mounted.current) setBusy(false);
    if (id) {
      try { const ended = await transport.stop(id); if (mounted.current) setSnapshot(current => current?.id === id ? ended : current); return ended; }
      catch (err) { if (mounted.current) setError(message(err)); }
    }
  }, [transport, clearCapture]);
  const fail = useCallback((err: unknown) => { if (mounted.current) setError(message(err)); void stop(); }, [stop]);
  const refresh = useCallback(async (id: string) => {
    if (refreshing.current) return;
    refreshing.current = true;
    try {
      const next = await transport.snapshot(id);
      if (active.current !== id || !mounted.current) return;
      setSnapshot(current => current?.id === id && current.sequence > next.sequence ? current : next);
      if (liveEnded(next.phase)) { active.current = null; generation.current++; clearCapture(); setBusy(false); if (next.error) setError(next.error); }
    } catch (err) { if (active.current === id && !transport.isTransientError?.(err)) fail(err); }
    finally { refreshing.current = false; }
  }, [transport, clearCapture, fail]);
  useEffect(() => transport.connection?.(state => {
    connected.current = state === 'connected'; setReconnecting(state === 'reconnecting');
    if (state === 'closed') { fail(new Error('The remote device was disconnected. Pair again to start a new Live session.')); return; }
    if (!active.current) return;
    for (const track of media.current?.getTracks() ?? []) track.enabled = connected.current;
    if (reconnectDeadline.current) clearTimeout(reconnectDeadline.current); reconnectDeadline.current = null;
    if (connected.current) { audioQueue.current?.resume(); void refresh(active.current); }
    else { audioQueue.current?.pause(); reconnectDeadline.current = setTimeout(() => { if (!connected.current && active.current) fail(new Error('Live reconnection expired. Reconnect and start capture again.')); }, 30_000); }
    captureTransition.current = captureTransition.current.then(async () => { if (!active.current) return; if (connected.current) await voiceRef.current.resumeRecording(); else await voiceRef.current.pauseRecording(); }).catch(fail);
  }), [transport, refresh, fail]);
  useEffect(() => {
    mounted.current = true; let disposed = false; let unlisten: (() => void) | undefined;
    void transport.subscribe(event => {
      if (active.current !== event.sessionId) return;
      setSnapshot(current => {
        if (!current) return current;
        if (event.sequence > current.sequence + 1) void refresh(event.sessionId);
        return applyLiveEvent(current, event);
      });
      if (event.type === 'state' && liveEnded(event.phase)) { active.current = null; generation.current++; clearCapture(); setBusy(false); if (event.error) setError(event.error); }
    }).then(dispose => { if (disposed) dispose(); else unlisten = dispose; }).catch(err => { if (!disposed) setError(message(err)); });
    const heartbeat = setInterval(() => { if (active.current && connected.current) void refresh(active.current); }, 10_000);
    const leave = () => { void stop(); };
    window.addEventListener('pagehide', leave);
    return () => { mounted.current = false; disposed = true; unlisten?.(); clearInterval(heartbeat); window.removeEventListener('pagehide', leave); void stop(); };
  }, [transport, refresh, clearCapture, stop]);
  const start = useCallback(async (request: StartLiveRequest, source: LiveVideoSource) => {
    if (active.current || starting.current) return;
    starting.current = true;
    const mine = ++generation.current;
    setBusy(true); setError(''); setSnapshot(null);
    let pendingMedia: MediaStream | null = null;
    try {
      if (!window.isSecureContext || !navigator.mediaDevices) throw new Error('Microphone and camera require a trusted HTTPS connection or localhost. Open the HTTPS pairing address to use Live.');
      // Display capture must be requested within the original click's activation.
      if (source !== 'none') pendingMedia = source === 'screen'
        ? await navigator.mediaDevices.getDisplayMedia({ video: true, audio: false })
        : await navigator.mediaDevices.getUserMedia({ video: { width: { ideal: 960 }, facingMode: { ideal: 'environment' } }, audio: false });
      if (generation.current !== mine) { pendingMedia?.getTracks().forEach(track => track.stop()); return; }
      media.current = pendingMedia; setPreview(pendingMedia);
      const opened = await transport.start(request);
      if (generation.current !== mine) { await transport.stop(opened.id); return; }
      active.current = opened.id; setSnapshot(opened);
      let ready = opened;
      const deadline = Date.now() + 25_000;
      while (ready.phase === 'connecting') {
        if (generation.current !== mine) return;
        if (Date.now() > deadline) throw new Error('The Live model did not become ready. Reconnect to continue.');
        await new Promise(resolve => setTimeout(resolve, 200));
        ready = await transport.snapshot(opened.id);
      }
      if (generation.current !== mine) return;
      if (liveEnded(ready.phase)) throw new Error(ready.error || 'Live session ended before capture started');
      setSnapshot(ready);
      if (request.microphone) {
        const queue = new LiveAudioQueue(Math.round(ready.sampleRate / 10) * 2, bytes => transport.audio(opened.id, bytes), fail);
        audioQueue.current = queue;
        await voiceRef.current.startRecording({ targetSampleRate: ready.sampleRate, onPcmChunk: chunk => queue.append(chunk), onCaptureIssue: () => fail(new Error('Microphone disconnected. Reconnect Live to continue.')) });
      }
      if (generation.current !== mine) { clearCapture(); return; }
      if (pendingMedia) {
        const element = document.createElement('video'); element.muted = true; element.playsInline = true; element.srcObject = pendingMedia;
        video.current = element; await element.play();
        for (const track of pendingMedia.getTracks()) track.onended = () => { void stop(); };
        let sending = false;
        timer.current = setInterval(() => {
          if (sending || generation.current !== mine || !connected.current) return;
          sending = true;
          void captureFrame(element).then(data => {
            if (data && generation.current === mine) return transport.frame(opened.id, 'image/jpeg', data);
          }).catch(err => { if (generation.current === mine) fail(err); }).finally(() => { sending = false; });
        }, Math.max(1, request.intervalSeconds) * 1000);
      }
      if (generation.current !== mine) { clearCapture(); return; }
      starting.current = false;
      if (mounted.current) setBusy(false);
    } catch (err) { if (generation.current === mine) fail(err); else pendingMedia?.getTracks().forEach(track => track.stop()); }
  }, [transport, clearCapture, fail, stop]);
  return { snapshot, setSnapshot, busy, error, setError, preview, start, stop, reconnecting, active: busy || Boolean(snapshot && !liveEnded(snapshot.phase)) };
}
