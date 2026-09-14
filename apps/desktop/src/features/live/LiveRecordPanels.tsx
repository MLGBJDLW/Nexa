import { useEffect, useRef, type ReactNode } from 'react';
import { Activity, FileText, Maximize2, Mic, X } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { StreamingMarkdown } from '../../components/chat/StreamingMarkdown';
import { observeScrollFollow } from '../../lib/scrollFollow';
import type { LiveEntry, LiveSnapshot } from './liveTransport';

function EntryPanel({ entries, speech }: { entries: LiveEntry[]; speech: boolean }) {
  const { t } = useTranslation();
  const scroller = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  useEffect(() => {
    if (!scroller.current || !content.current) return;
    return observeScrollFollow(scroller.current, content.current, following, () => {});
  }, []);
  return <section className="min-w-0 overflow-hidden rounded-xl border border-border bg-surface-1/60" data-testid={speech ? 'live-speech-panel' : 'live-observation-panel'}>
    <header className="flex items-center gap-2 border-b border-border px-4 py-3 text-sm font-medium">
      {speech ? <Mic size={15} className="text-accent" /> : <Activity size={15} className="text-accent" />}
      <h2>{t(speech ? 'live.transcriptTitle' : 'live.observation')}</h2>
      <span className="ml-auto text-xs tabular-nums text-text-tertiary">{entries.length}</span>
    </header>
    {speech && <p className="px-4 pt-3 text-xs text-text-tertiary">{t('live.speakerUnavailable')}</p>}
    <div ref={scroller} className="max-h-80 min-h-28 overflow-y-auto p-4 [overflow-anchor:none] [scrollbar-width:thin]" aria-live="polite" aria-relevant="additions text">
      <div ref={content} className="space-y-4">
        {!entries.length && <p className="py-5 text-sm text-text-tertiary">{t(speech ? 'live.speechEmpty' : 'live.observationEmpty')}</p>}
        {entries.map(entry => <article key={entry.id} className="grid grid-cols-[42px_minmax(0,1fr)] gap-3">
          <time className="pt-0.5 text-xs tabular-nums text-text-tertiary">{Math.floor(entry.atMs / 60000)}:{String(Math.floor(entry.atMs / 1000) % 60).padStart(2, '0')}</time>
          <div className="min-w-0">
            {!entry.complete && <span className="mb-1 block text-[11px] text-text-tertiary">{t('live.partial')}</span>}
            <p className="whitespace-pre-wrap break-words text-sm leading-relaxed">{entry.text}</p>
          </div>
        </article>)}
      </div>
    </div>
  </section>;
}

export function LiveRecordPanels({ snapshot, summary, controls, summarizing }: {
  snapshot: LiveSnapshot | null; summary: string; controls: ReactNode; summarizing: boolean;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDialogElement>(null);
  const entries = snapshot?.entries ?? [];
  const notices = entries.filter(entry => entry.role === 'notice');
  const summaryContent = summary
    ? <StreamingMarkdown content={summary} isStreaming={false} reduceMotion />
    : <p className="text-sm leading-relaxed text-text-tertiary">{t(summarizing ? 'live.summaryWorking' : 'live.summaryEmpty')}</p>;
  return <div className="grid min-w-0 items-start gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(280px,0.9fr)]" data-testid="live-record-panels">
    <div className="min-w-0 space-y-4">
      <EntryPanel key={`${snapshot?.id}:speech`} entries={entries.filter(entry => entry.role === 'user')} speech />
      <EntryPanel key={`${snapshot?.id}:observations`} entries={entries.filter(entry => entry.role !== 'user' && entry.role !== 'notice')} speech={false} />
      {notices.length > 0 && <details className="rounded-xl border border-border px-4 py-3 text-xs text-text-tertiary"><summary className="cursor-pointer">{t('live.captureStatus')} · {notices.length}</summary><div className="mt-3 space-y-2">{notices.map(entry => <p key={entry.id}>{entry.text}</p>)}</div></details>}
      {Boolean(snapshot?.metrics.omittedEntries) && <p className="text-xs text-text-tertiary">{t('live.omitted')}: {snapshot?.metrics.omittedEntries}</p>}
    </div>
    <section className="min-w-0 rounded-xl border border-border bg-surface-1/60 xl:sticky xl:top-4" data-testid="live-summary-panel">
      <header className="flex items-center gap-2 border-b border-border px-4 py-3 text-sm font-medium"><FileText size={15} className="text-accent" /><h2>{t('live.summaryTitle')}</h2><button type="button" className="ml-auto rounded p-1.5 text-text-tertiary hover:bg-surface-2 hover:text-text-primary focus-visible:outline-2 focus-visible:outline-accent" aria-label={t('live.expandSummary')} onClick={() => dialog.current?.showModal()}><Maximize2 size={15} /></button></header>
      <div className="space-y-4 p-4">{controls}<div className="max-h-[55vh] overflow-y-auto break-words text-sm" data-testid="live-summary" aria-busy={summarizing}>{summaryContent}</div></div>
    </section>
    <dialog ref={dialog} aria-labelledby="live-summary-dialog-title" className="m-auto max-h-[85dvh] w-[min(900px,calc(100vw-2rem))] rounded-2xl border border-border bg-surface-1 p-0 text-text-primary shadow-xl backdrop:bg-black/45" data-testid="live-summary-dialog">
      <header className="flex items-center justify-between border-b border-border px-6 py-4"><h2 id="live-summary-dialog-title" className="text-lg font-semibold">{t('live.summaryTitle')}</h2><button type="button" className="rounded p-2 hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-accent" aria-label={t('common.close')} onClick={() => dialog.current?.close()}><X size={18} /></button></header>
      <div className="max-h-[calc(85dvh-80px)] overflow-y-auto break-words p-6 leading-relaxed" aria-busy={summarizing}>{summaryContent}</div>
    </dialog>
  </div>;
}
