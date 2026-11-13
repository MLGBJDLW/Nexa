import { useEffect, useRef, useState } from 'react';
import { ArrowLeft, ArrowDown, ArrowUp, Home, Monitor, Pause, Play, Send, X } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { remoteButton, remoteField } from './remoteUi';

export interface ScreenFrame {
  observationId: string; windowId?: number; packageName?: string; title?: string; controlAvailable?: boolean;
  image?: { base64: string; mimeType: string; width: number; height: number };
  nodes?: { nodeId: number; focused: boolean; editable: boolean; scrollable: boolean }[];
  code?: string; message?: string;
}
export interface ScreenTransport { phone?: boolean; frame(): Promise<ScreenFrame>; input(value: Record<string, unknown>): Promise<unknown>; stop?(): Promise<unknown> }

/** One frame in flight, no background backlog and no automatic input replay. */
export function DeviceScreen({ transport, onClose }: { transport: ScreenTransport; onClose?: () => void }) {
  const { t } = useTranslation();
  const [frame, setFrame] = useState<ScreenFrame | null>(null);
  const [active, setActive] = useState(true);
  const [control, setControl] = useState(false);
  const [text, setText] = useState('');
  const [error, setError] = useState('');
  const [captureError, setCaptureError] = useState('');
  const [fresh, setFresh] = useState(false);
  const [acting, setActing] = useState(false);
  const [zoom, setZoom] = useState(1);
  const working = useRef(false);
  const capturing = useRef<Promise<ScreenFrame> | null>(null);
  const inputPending = useRef(false);
  const captureValid = useRef(false);
  const pointer = useRef({x:0,y:0});
  const scrollTarget = useRef({x:0.5,y:0.5});
  const current = useRef(frame); current.current = frame;
  const generation = useRef(0);
  const refresh = useRef<() => Promise<void>>(async () => {});
  useEffect(() => {
    setFrame(null); setControl(false); setText(''); setError(''); setCaptureError(''); setFresh(false); setActing(false); setActive(true); setZoom(1);
    current.current = null; captureValid.current = false;
    scrollTarget.current = {x:0.5,y:0.5};
  }, [transport]);
  useEffect(() => {
    const mine = ++generation.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const capture = async () => {
      clearTimeout(timer);
      if (mine !== generation.current || !active) return;
      if (!working.current && !inputPending.current && document.visibilityState === 'visible') {
        working.current = true;
        try {
          const next = await (capturing.current = transport.frame());
          if (mine !== generation.current) return;
          if (!next.image) throw new Error(next.message || t('remote.screenUnavailable'));
          setFrame(next); setCaptureError(''); setFresh(true); captureValid.current = true;
        } catch (error) { if (mine === generation.current) { setCaptureError(String(error)); setFresh(false); captureValid.current = false; } }
        finally { working.current = false; capturing.current = null; }
      }
      if (mine === generation.current) timer = setTimeout(capture, 1000);
    };
    refresh.current = capture;
    const visible = () => { if (document.visibilityState === 'visible') void capture(); };
    document.addEventListener('visibilitychange', visible);
    void capture();
    return () => { generation.current++; clearTimeout(timer); document.removeEventListener('visibilitychange', visible); };
  }, [transport, active, t]);
  const send = async (action: string, values: Record<string, unknown> = {}) => {
    const observed = current.current;
    if (!control || !active || !observed || !fresh) return;
    if (inputPending.current) { setError(t('remote.screenBusy')); return; }
    inputPending.current = true; setActing(true); setError('');
    const mine = generation.current;
    try {
      // Give a human action priority over the next capture, preserving the
      // observation the user clicked. Never drop clicks during a slow frame.
      await capturing.current?.catch(() => {});
      if (mine !== generation.current || !captureValid.current) return;
      working.current = true;
      const result = await transport.input(transport.phone
        ? { action, observationId: observed.observationId, packageName: observed.packageName, ...values }
        : { action, observation_id: observed.observationId, window_id: observed.windowId, ...values });
      if (result && typeof result === 'object' && 'code' in result) throw new Error(String((result as ScreenFrame).message || (result as ScreenFrame).code));
      if (mine === generation.current && (action === 'type_text' || action === 'set_text')) setText('');
    } catch (error) { if (mine === generation.current) setError(String(error)); }
    finally { working.current = false; inputPending.current = false; if (mine === generation.current) { setActing(false); void refresh.current(); } }
  };
  const scrollDesktop = (direction: number) => {
    const image = current.current?.image;
    if (!image) return;
    void send('scroll', {scroll_y: direction * 3, x: Math.min(image.width - 1, scrollTarget.current.x * image.width), y: Math.min(image.height - 1, scrollTarget.current.y * image.height)});
  };
  const stop = () => { generation.current++; setActive(false); setControl(false); setFrame(null); void transport.stop?.().catch(error => setError(String(error))); onClose?.(); };
  const scrollNode = frame?.nodes?.find(node => node.scrollable)?.nodeId;
  const editor = frame?.nodes?.find(node => node.focused && node.editable)?.nodeId;
  const button = `${remoteButton} min-h-11 min-w-11`;
  return <section className="space-y-3 rounded-xl border border-border bg-surface-1 p-3" data-testid="device-screen">
    <div className="flex flex-wrap items-center gap-2">
      <Monitor size={17}/><h3 className="mr-auto min-w-0 truncate text-sm font-medium">{frame?.title || t('remote.screenTitle')}</h3>
      <button className={button} onClick={() => { captureValid.current = false; setFresh(false); setActive(value => !value); }} aria-label={t(active ? 'remote.screenPause' : 'remote.screenResume')}>{active ? <Pause size={16}/> : <Play size={16}/>}</button>
      <button className={button} onClick={() => setZoom(value => value === 1 ? 2 : 1)} aria-label={t('remote.screenZoom')}>{zoom}×</button>
      <button className={button} onClick={stop} aria-label={t('remote.screenStop')}><X size={16}/></button>
    </div>
    <label className="flex items-center gap-2 text-xs"><input type="checkbox" checked={control} onChange={event => setControl(event.target.checked)} disabled={!active || !fresh || frame?.controlAvailable === false}/>{t('remote.screenControl')}</label>
    {(error || captureError) && <p role="alert" className="break-words text-xs text-danger">{error || captureError}</p>}
    <div className={`relative min-h-40 max-h-[65dvh] overflow-auto rounded-lg bg-black ${acting ? 'ring-2 ring-accent' : ''}`}>
      <div className={`grid min-h-40 min-w-full place-items-center ${zoom === 1 ? 'w-full' : 'w-max'}`}>
      {frame?.image ? <img alt={t('remote.screenTitle')} draggable={false} src={`data:${frame.image.mimeType};base64,${frame.image.base64}`}
        className={`block select-none object-contain ${zoom === 1 ? 'max-h-[65dvh] max-w-full' : 'max-w-none'} ${control && fresh ? 'cursor-crosshair' : ''}`}
        style={zoom > 1 ? {width:frame.image.width} : undefined}
        onPointerDown={event => {pointer.current={x:event.clientX,y:event.clientY};}}
        onClick={event => { if(Math.abs(event.clientX-pointer.current.x)+Math.abs(event.clientY-pointer.current.y)>10) return; const bounds = event.currentTarget.getBoundingClientRect(); const image = frame.image!; const x = Math.min(image.width - 1, Math.max(0, (event.clientX-bounds.left)*image.width/bounds.width)); const y = Math.min(image.height - 1, Math.max(0, (event.clientY-bounds.top)*image.height/bounds.height)); scrollTarget.current = {x:x/image.width,y:y/image.height}; void send(transport.phone ? 'tap' : 'click', {x,y}); }}/>
        : <p className="p-5 text-center text-sm text-white/70">{t('remote.screenWaiting')}</p>}
      </div>
    </div>
    {control && <><div className="flex flex-wrap gap-2">
      {transport.phone && <><button className={button} disabled={acting} onClick={() => void send('back')} aria-label={t('remote.screenBack')}><ArrowLeft size={16}/></button><button className={button} disabled={acting} onClick={() => void send('home')} aria-label={t('remote.screenHome')}><Home size={16}/></button></>}
      {!transport.phone && <button className={button} disabled={acting} onClick={() => void send('focus_window')}>{t('remote.screenFocus')}</button>}
      {!transport.phone && ['ENTER','TAB','ESC'].map(key => <button key={key} className={button} disabled={acting} onClick={() => void send('key',{key_sequence:key})}>{key}</button>)}
      {[-1, 1].map(direction => <button key={direction} className={button} disabled={acting} aria-label={t(direction < 0 ? 'remote.screenUp' : 'remote.screenDown')} onClick={() => { if (transport.phone) void send(direction < 0 ? 'scroll_backward' : 'scroll_forward', {nodeId:scrollNode}); else scrollDesktop(direction); }}>{direction < 0 ? <ArrowUp size={16}/> : <ArrowDown size={16}/>}</button>)}
    </div><form className="flex gap-2" onSubmit={event => { event.preventDefault(); void send(transport.phone ? 'set_text' : 'type_text', {text, ...(transport.phone && editor != null ? {nodeId:editor} : {})}); }}>
      <input className={remoteField} value={text} onChange={event => setText(event.target.value)} placeholder={t('remote.screenType')} aria-label={t('remote.screenType')}/>
      <button className={button} disabled={acting || !text} aria-label={t('chat.send')}><Send size={16}/></button>
    </form></>}
    <p className="text-xs leading-5 text-text-tertiary">{t('remote.screenHint')}</p>
  </section>;
}
