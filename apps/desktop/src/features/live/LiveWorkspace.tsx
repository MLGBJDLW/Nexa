import { useEffect, useRef, useState } from 'react';
import { Activity, Camera, CircleStop, FileText, Loader2, MessageCircle, Mic, Radio } from 'lucide-react';
import { useTranslation, type TranslationKey } from '../../i18n';
import type { LiveConnection, LiveProtocol, LiveRecordRef, LiveTransport } from './liveTransport';
import { useLiveSession, type LiveVideoSource } from './useLiveSession';

const models: Record<LiveProtocol, string> = { openAiRealtime: 'gpt-realtime-2.1', geminiLive: 'gemini-3.1-flash-live-preview', qwenRealtime: 'qwen3.5-omni-flash-realtime' };
const protocolNames: Record<LiveProtocol, string> = { openAiRealtime: 'OpenAI Realtime', geminiLive: 'Gemini Live', qwenRealtime: 'Qwen Omni Realtime' };
const field = 'w-full rounded-lg border border-border bg-surface-1 px-3 py-2 text-sm text-text-primary outline-none focus:ring-2 focus:ring-accent/50 disabled:opacity-50';
const button = 'inline-flex items-center justify-center gap-2 rounded-lg border border-border px-3 py-2 text-sm font-medium transition-colors hover:bg-surface-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-40 disabled:pointer-events-none';

export function LiveWorkspace({ transport, onSendToChat }: { transport: LiveTransport; onSendToChat: (text: string) => void }) {
  const { t } = useTranslation();
  const live = useLiveSession(transport);
  const [connections, setConnections] = useState<LiveConnection[]>([]);
  const [connectionId, setConnectionId] = useState('');
  const [summaryId, setSummaryId] = useState('');
  const [protocol, setProtocol] = useState<LiveProtocol | ''>('');
  const [model, setModel] = useState('');
  const [endpoint, setEndpoint] = useState('');
  const [microphone, setMicrophone] = useState(true);
  const [source, setSource] = useState<LiveVideoSource>('none');
  const [purpose, setPurpose] = useState('');
  const [interval, setInterval] = useState(2);
  const [summary, setSummary] = useState('');
  const [summarizing, setSummarizing] = useState(false);
  const [history, setHistory] = useState<LiveRecordRef[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadRevision, setLoadRevision] = useState(0);
  const preview = useRef<HTMLVideoElement>(null);
  const mounted = useRef(true);
  const selected = connections.find(item => item.id === connectionId);
  useEffect(() => {
    let cancelled = false; mounted.current = true; setLoading(true);
    void Promise.all([transport.connections(), transport.list()]).then(([items, records]) => {
      if (cancelled) return;
      setConnections(items); setHistory(records);
      const initial = items.find(item => item.isDefault) || items[0];
      setConnectionId(current => current || initial?.id || ''); setSummaryId(current => current || initial?.id || '');
    }).catch(error => { if (!cancelled) live.setError(String(error)); }).finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; mounted.current = false; };
  }, [transport, loadRevision, live.setError]);
  useEffect(() => { if (preview.current) preview.current.srcObject = live.preview; }, [live.preview]);
  async function stop() {
    await live.stop();
    try { const records = await transport.list(); if (mounted.current) setHistory(records); } catch (error) { if (mounted.current) live.setError(String(error)); }
  }
  async function summarize() {
    if (!live.snapshot) return;
    const id = live.snapshot.id;
    setSummarizing(true); live.setError('');
    try { const record = await transport.summarize(id, summaryId); if (mounted.current) setSummary(record.summary || ''); }
    catch (error) { if (mounted.current) live.setError(String(error)); }
    finally { if (mounted.current) setSummarizing(false); }
  }
  const status = live.snapshot?.phase ?? 'stopped';
  const evidence = summary || live.snapshot?.entries.map(entry => `[${Math.floor(entry.atMs / 1000)}s · ${entry.role}${entry.complete ? '' : ' · partial'}] ${entry.text}`).join('\n') || '';
  return <div className="mx-auto flex w-full max-w-[1440px] flex-col gap-6 p-4 text-text-primary sm:p-7" data-testid="live-workspace">
    <header className="flex flex-wrap items-start justify-between gap-4">
      <div><div className="mb-2 flex items-center gap-2 text-xs font-semibold tracking-[0.14em] text-accent"><Radio size={15} /> NEXA LIVE</div><h1 className="text-2xl font-semibold">{t('live.title')}</h1><p className="mt-2 max-w-2xl text-sm leading-relaxed text-text-secondary">{t('live.subtitle')}</p></div>
      <span role="status" className="flex items-center gap-2 rounded-lg border border-border bg-surface-1 px-3 py-2 text-sm"><span className={`h-2 w-2 rounded-full ${live.active ? 'bg-success motion-safe:animate-pulse' : 'bg-text-tertiary'}`} />{t(`live.${status}` as TranslationKey)}</span>
    </header>
    {live.error && <div role="alert" className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-danger/30 bg-danger/5 p-3 text-sm text-danger"><span>{live.error}</span>{!live.active && <button className={button} onClick={() => { live.setError(''); setLoadRevision(value => value + 1); }}>{t('common.retry')}</button>}</div>}
    <div className="grid items-start gap-6 xl:grid-cols-[320px_minmax(0,1fr)]">
      <section className="space-y-4 rounded-xl border border-border bg-surface-1/70 p-5" aria-label={t('live.setup')}>
        <h2 className="flex items-center gap-2 font-medium"><Activity size={17} className="text-accent" />{t('live.setup')}</h2>
        <details open={!live.active} className="space-y-4">
          <summary className="cursor-pointer text-sm text-text-secondary">{protocol ? protocolNames[protocol] : t('live.incremental')} · {live.snapshot?.model || selected?.model || '…'}</summary>
        <fieldset disabled={live.active || loading} className="space-y-4">
          <label className="block space-y-1.5 text-xs font-medium text-text-secondary"><span>{t('live.observer')}</span><select className={field} value={connectionId} onChange={event => { setConnectionId(event.target.value); setProtocol(''); }}><option value="" disabled>{loading ? t('live.connecting') : t('live.selectConnection')}</option>{connections.map(item => <option key={item.id} value={item.id}>{item.name} · {item.model}</option>)}</select></label>
          <label className="block space-y-1.5 text-xs font-medium text-text-secondary"><span>{t('live.mode')}</span><select className={field} value={protocol} onChange={event => { const value = event.target.value as LiveProtocol | ''; setProtocol(value); setModel(value ? models[value] : ''); }}><option value="">{t('live.incremental')}</option>{selected?.nativeProtocols.map(value => <option key={value} value={value}>{protocolNames[value]}</option>)}</select></label>
          <p className="text-xs leading-relaxed text-text-tertiary">{protocol ? t('live.nativeHint') : t('live.incrementalHint')}</p>
          {protocol && <label className="block space-y-1.5 text-xs font-medium text-text-secondary"><span>{t('live.model')}</span><input className={field} value={model} maxLength={160} onChange={event => setModel(event.target.value)} /></label>}
          {protocol === 'qwenRealtime' && <label className="block space-y-1.5 text-xs font-medium text-text-secondary"><span>{t('live.endpoint')}</span><input aria-label={t('live.endpoint')} className={field} value={endpoint} placeholder="wss://<WorkspaceId>.cn-beijing.maas.aliyuncs.com/api-ws/v1/realtime" onChange={event => setEndpoint(event.target.value)} /><span className="block text-xs font-normal leading-relaxed text-text-tertiary">{t('live.endpointHint')}</span></label>}
          <div className="grid grid-cols-2 gap-3">
            <label className="flex items-center gap-2 rounded-lg border border-border px-3 py-2.5 text-sm"><input type="checkbox" checked={microphone} onChange={event => setMicrophone(event.target.checked)} /><Mic size={15} />{t('live.microphone')}</label>
            <label className="block text-xs text-text-secondary"><span className="sr-only">{t('live.visualInput')}</span><select aria-label={t('live.visualInput')} className={field} value={source} onChange={event => setSource(event.target.value as LiveVideoSource)}><option value="none">{t('live.none')}</option><option value="camera">{t('live.camera')}</option>{typeof navigator.mediaDevices?.getDisplayMedia === 'function' && <option value="screen">{t('live.screen')}</option>}</select></label>
          </div>
          <label className="block space-y-1.5 text-xs font-medium text-text-secondary"><span>{t('live.purpose')}</span><textarea className={`${field} resize-y`} rows={3} maxLength={4000} value={purpose} placeholder={t('live.purposeHint')} onChange={event => setPurpose(event.target.value)} /></label>
          <label className="flex items-center justify-between gap-3 text-xs font-medium text-text-secondary"><span>{t('live.interval')}</span><input aria-label={t('live.interval')} className={`${field} !w-20`} type="number" min={1} max={30} value={interval} onChange={event => setInterval(Math.min(30, Math.max(1, Number(event.target.value) || 1)))} /></label>
        </fieldset>
        </details>
        {live.active ? <button className={`${button} w-full border-danger/40 text-danger`} onClick={() => void stop()}><CircleStop size={17} />{t('live.stop')}</button> : <button className={`${button} w-full !border-accent !bg-accent text-white`} disabled={!connectionId || loading || (!microphone && source === 'none') || (protocol === 'qwenRealtime' && (!microphone || !endpoint.trim()))} onClick={() => { setSummary(''); void live.start({ connectionId, protocol: protocol || null, nativeModel: model || null, nativeEndpoint: endpoint || null, microphone, images: source !== 'none', purpose, intervalSeconds: interval }, source); }}><Radio size={17} />{t('live.start')}</button>}
        <p className="text-xs leading-relaxed text-text-tertiary">{t('live.privacy')}</p>
      </section>
      <div className="min-w-0 space-y-5">
        {live.preview && <div className="overflow-hidden rounded-xl border border-border bg-black"><video ref={preview} autoPlay muted playsInline aria-label={t('live.preview')} className="max-h-72 w-full object-contain" /></div>}
        <section className="overflow-hidden rounded-xl border border-border bg-surface-1/60">
          <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border px-5 py-3"><h2 className="flex items-center gap-2 text-sm font-medium"><FileText size={16} />{t('live.observations')}</h2><div className="flex gap-4 text-xs tabular-nums text-text-tertiary"><span>{t('live.frames')}: {live.snapshot?.metrics.framesSubmitted ?? 0}</span><span>{t('live.skipped')}: {live.snapshot?.metrics.framesReplaced ?? 0}</span><span>{t('live.latency')}: {live.snapshot?.metrics.lastResponseMs == null ? '—' : `${(live.snapshot.metrics.lastResponseMs / 1000).toFixed(1)}s`}</span></div></div>
          <div className="max-h-[540px] min-h-64 space-y-4 overflow-y-auto p-5" aria-live="polite" aria-relevant="additions text">
            {!live.snapshot?.entries.length && <div className="flex min-h-52 flex-col items-center justify-center gap-3 text-center text-text-tertiary">{live.busy ? <Loader2 className="animate-spin" size={28} /> : <Camera size={28} strokeWidth={1.4} />}<p className="max-w-md text-sm leading-relaxed">{live.busy ? t('live.connecting') : t('live.empty')}</p></div>}
            {live.snapshot?.entries.map(entry => <article key={entry.id} className="grid grid-cols-[54px_minmax(0,1fr)] gap-3"><span className="pt-0.5 text-xs tabular-nums text-text-tertiary">{Math.floor(entry.atMs / 60000)}:{String(Math.floor(entry.atMs / 1000) % 60).padStart(2, '0')}</span><div><div className="mb-1 flex items-center gap-2 text-xs font-medium text-text-tertiary">{entry.role === 'user' ? <Mic size={12} /> : <Activity size={12} />}{entry.role === 'user' ? t('live.speech') : t('live.observation')}{!entry.complete && <span className="font-normal">· {t('live.partial')}</span>}</div><p className="whitespace-pre-wrap break-words text-sm leading-relaxed">{entry.text}</p></div></article>)}
          </div>
          {Boolean(live.snapshot?.metrics.omittedEntries) && <p className="border-t border-border px-5 py-2 text-xs text-text-tertiary">{t('live.omitted')}: {live.snapshot?.metrics.omittedEntries}</p>}
        </section>
        <section className="space-y-3 rounded-xl border border-border bg-surface-1/60 p-5">
          <div className="flex flex-wrap items-end gap-3"><label className="min-w-40 flex-1 space-y-1.5 text-xs font-medium text-text-secondary"><span>{t('live.summaryModel')}</span><select className={field} value={summaryId} disabled={summarizing} onChange={event => setSummaryId(event.target.value)}>{connections.map(item => <option key={item.id} value={item.id}>{item.name} · {item.model}</option>)}</select></label><button className={button} disabled={live.active || !live.snapshot?.entries.length || !summaryId || summarizing} onClick={() => void summarize()}>{summarizing && <Loader2 size={15} className="animate-spin" />}{t('live.summarize')}</button><button className={button} disabled={live.active || !evidence || summarizing} onClick={() => onSendToChat(`${t('live.handoff')}\n\n${evidence}`)}><MessageCircle size={15} />{t('live.sendToChat')}</button></div>
          {summary && <p className="whitespace-pre-wrap break-words text-sm leading-relaxed" data-testid="live-summary">{summary}</p>}
        </section>
        <section className="space-y-2"><h2 className="text-sm font-medium text-text-secondary">{t('live.history')}</h2>{history.length ? <div className="grid gap-2 sm:grid-cols-2">{history.map(record => <button key={record.id} className={`${button} !justify-start text-left`} disabled={live.active || summarizing} onClick={() => void transport.load(record.id).then(result => { live.setSnapshot(result.snapshot); setSummary(result.summary || ''); }).catch(error => live.setError(String(error)))}><FileText className="shrink-0 text-text-tertiary" size={16} /><span className="min-w-0"><span className="block truncate">{record.model}</span><span className="block text-xs font-normal text-text-tertiary">{new Date(record.startedAt).toLocaleString()}</span></span></button>)}</div> : <p className="text-xs text-text-tertiary">{t('live.noHistory')}</p>}</section>
      </div>
    </div>
  </div>;
}
