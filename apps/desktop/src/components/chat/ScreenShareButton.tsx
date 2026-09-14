import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { MonitorUp, Square, Loader2 } from 'lucide-react';
import { toast } from 'sonner';
import { useTranslation } from '../../i18n';
import { encodeSharedScreenFrame } from '../../lib/sharedScreenFrame';
import { openScreenCapture, useDesktopMonitors, type ScreenCapture } from '../../lib/screenCapture';
import { NexaPopover, NexaPopoverTrigger, NexaPopoverContent } from '../ui/overlay/Popover';

interface Capture {
  conversationId: string; lease: string; stream: MediaStream;
  video: HTMLVideoElement; timer?: ReturnType<typeof setInterval>; close(): void;
}

export function ScreenShareButton({ conversationId }: { conversationId?: string }) {
  const { t } = useTranslation();
  const capture = useRef<Capture | null>(null);
  const pendingStream = useRef<MediaStream | null>(null);
  const generation = useRef(0);
  const mounted = useRef(true);
  const [pending, setPending] = useState(false);
  const [sharing, setSharing] = useState(false);
  const [preview, setPreview] = useState('');
  const [pickerOpen, setPickerOpen] = useState(false);
  const browserSupported = typeof navigator.mediaDevices?.getDisplayMedia === 'function';
  const monitors = useDesktopMonitors(pickerOpen || !browserSupported);
  const stop = useCallback(() => {
    generation.current++;
    const current = capture.current;
    capture.current = null;
    pendingStream.current?.getTracks().forEach(track => track.stop());
    pendingStream.current = null;
    if (current) {
      clearInterval(current.timer);
      current.stream.getTracks().forEach(track => { track.onended = null; track.stop(); });
      current.close();
      current.video.pause(); current.video.srcObject = null;
      void invoke('end_desktop_share_cmd', { conversationId: current.conversationId, lease: current.lease }).catch(() => {});
    }
    if (mounted.current) { setPending(false); setSharing(false); setPreview(''); }
  }, []);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; stop(); }; }, [stop]);
  useEffect(() => { stop(); return stop; }, [conversationId, stop]);

  const start = async (monitorId?: string, sourceName?: string) => {
    if (!conversationId || pending || capture.current) return;
    const mine = ++generation.current;
    setPending(true);
    let stream: MediaStream | undefined;
    let lease: string | undefined;
    let session: ScreenCapture | undefined;
    try {
      session = await openScreenCapture(monitorId);
      stream = session.stream;
      if (mine !== generation.current) { session.stop(); return; }
      pendingStream.current = stream;
      const video = document.createElement('video');
      video.srcObject = stream; video.muted = true; video.playsInline = true;
      await video.play();
      lease = await invoke<string>('begin_desktop_share_cmd', { conversationId, source: sourceName || stream.getVideoTracks()[0]?.label || 'Screen' });
      if (!lease || typeof lease !== 'string') throw new Error(t('chat.screenShareUnavailable'));
      if (mine !== generation.current) {
        session.stop();
        void invoke('end_desktop_share_cmd', { conversationId, lease }).catch(() => {});
        return;
      }
      if (stream.getVideoTracks().every(track => track.readyState === 'ended')) throw new Error(t('chat.screenShareUnavailable'));
      const current: Capture = { conversationId, lease, stream, video, close: session.stop };
      capture.current = current;
      pendingStream.current = null;
      let sequence = 0;
      let uploading = false;
      const readyDeadline = Date.now() + 5000;
      const canvas = document.createElement('canvas');
      const upload = async () => {
        if (uploading || mine !== generation.current) return;
        if (!video.videoWidth || !video.videoHeight) {
          if (Date.now() > readyDeadline) { stop(); toast.error(t('chat.screenShareUnavailable')); }
          return;
        }
        uploading = true;
        try {
          const frame = encodeSharedScreenFrame(video, video.videoWidth, video.videoHeight, canvas);
          if (!frame) return; // A bad frame must not terminate an otherwise valid share.
          const { url, base64 } = frame;
          await invoke('update_desktop_share_cmd', { conversationId, lease, sequence: ++sequence, base64 });
          if (mine === generation.current && mounted.current) { setPreview(url); setSharing(true); setPending(false); }
        } catch (error) {
          if (mine === generation.current) { stop(); toast.error(String(error)); }
        } finally { uploading = false; }
      };
      stream.getVideoTracks().forEach(track => { track.onended = stop; });
      current.timer = setInterval(() => void upload(), 1000);
      await upload();
    } catch (error) {
      stream?.getTracks().forEach(track => track.stop());
      session?.stop();
      if (lease) void invoke('end_desktop_share_cmd', { conversationId, lease }).catch(() => {});
      if (mine === generation.current) { stop(); if (!(error instanceof DOMException && error.name === 'NotAllowedError')) toast.error(String(error)); }
    }
  };
  const supported = browserSupported || monitors.length > 0;
  return <div className="group relative shrink-0" data-testid="desktop-share-control">
    <NexaPopover open={pickerOpen && !sharing && !pending} onOpenChange={setPickerOpen}>
    <NexaPopoverTrigger asChild>
    <button type="button" data-testid="desktop-share-toggle" aria-pressed={sharing} disabled={!conversationId || !supported}
      aria-label={t(sharing || pending ? 'chat.stopScreenShare' : 'chat.shareScreen')}
      title={t(!conversationId ? 'chat.screenShareNeedsConversation' : !supported ? 'chat.screenShareUnavailable' : sharing ? 'chat.screenShareActive' : 'chat.shareScreen')}
      onClick={event => { if (sharing || pending) { event.preventDefault(); stop(); setPickerOpen(false); } }}
      className={`flex h-8 items-center gap-1.5 rounded-md px-2 text-xs transition-colors disabled:opacity-40 ${sharing ? 'bg-accent/10 text-accent' : 'text-text-tertiary hover:bg-surface-2'}`}>
      {pending ? <Loader2 size={15} className="animate-spin" /> : sharing ? <Square size={13} /> : <MonitorUp size={15} />}
      {sharing && <span>{t('chat.screenShareActive')}</span>}
    </button>
    </NexaPopoverTrigger>
    <NexaPopoverContent side="top" align="start" aria-label={t('chat.shareScreen')} className="w-72 rounded-lg border border-border bg-surface-1 p-2 text-text-primary shadow-xl">
      <p className="px-2 py-1 text-xs text-text-tertiary">{t('chat.shareScreen')}</p>
      {monitors.map((monitor, index) => <button type="button" key={monitor.id} className="flex w-full items-center gap-2 rounded-md p-2 text-left text-sm hover:bg-surface-2" onClick={() => { setPickerOpen(false); void start(monitor.id, `${t('live.screen')} ${index + 1}`); }}>
        <MonitorUp size={16} /><span>{t('live.screen')} {index + 1}<span className="ml-2 text-xs text-text-tertiary">{monitor.width} × {monitor.height}</span></span>
      </button>)}
      {browserSupported && <button type="button" className="w-full rounded-md p-2 text-left text-sm hover:bg-surface-2" onClick={() => { setPickerOpen(false); void start(); }}>{t('chat.shareWindow')}</button>}
    </NexaPopoverContent>
    </NexaPopover>
    {sharing && preview && <div className="pointer-events-none absolute bottom-full left-0 z-40 mb-2 hidden w-60 overflow-hidden rounded-lg border border-border bg-surface-1 shadow-lg group-hover:block group-focus-within:block">
      <img src={preview} alt={t('chat.screenSharePreview')} className="w-full" /><p className="p-2 text-[11px] text-text-secondary">{t('chat.screenShareHint')}</p>
    </div>}
  </div>;
}
