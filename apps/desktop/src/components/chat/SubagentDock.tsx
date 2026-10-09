import { useEffect, useRef, useState, type PointerEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Bot, Loader2, RefreshCw, Send, Square, X } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { compactTaskLabel, type SubtaskRunArtifact } from '../../lib/taskArtifacts';
import { subagentRecord, SubagentJournalProjection, type SubagentWorkspacePage } from '../../lib/subagentWorkspace';
import { StreamingMarkdown } from './StreamingMarkdown';
import { ToolCallCard } from './ToolCallCard';

export function SubagentDock({ conversationId, subtask, onClose, onStatus, drafts }: {
  conversationId: string; subtask: SubtaskRunArtifact; onClose: () => void; onStatus: (id: string, status: string) => void; drafts: Map<string, string>;
}) {
  const { t } = useTranslation();
  const id = subtask.rowId || subtask.lifecycleId || subtask.id;
  const draftKey = `${conversationId}:${id}`;
  const [draft, setDraft] = useState(drafts.get(draftKey) ?? '');
  const [projection, setProjection] = useState(() => new SubagentJournalProjection());
  const [page, setPage] = useState<SubagentWorkspacePage | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  const [width, setWidth] = useState(600);
  const [visibleCount, setVisibleCount] = useState(100);
  const [truncated, setTruncated] = useState(false);
  const [, render] = useState(0);
  const closeRef = useRef<HTMLButtonElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const generation = useRef(0);

  useEffect(() => {
    const origin = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    closeRef.current?.focus();
    return () => { if (origin?.isConnected) origin.focus(); };
  }, []);
  useEffect(() => {
    let active = true;
    const unsubscribe = listen('privacy:revoked', () => {
      if (!active) return;
      generation.current += 1;
      setPage(null);
      setProjection(new SubagentJournalProjection()); setRevision(value => value + 1);
    });
    return () => { active = false; void unsubscribe.then(stop => stop()); };
  }, [drafts]);

  useEffect(() => {
    const serial = ++generation.current;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const journal = new SubagentJournalProjection();
    let policy: string | null = null;
    setProjection(journal); setPage(null); setError(''); setLoading(true); setTruncated(false);
    async function read() {
      try {
        const next = await invoke<SubagentWorkspacePage>('read_subagent_workspace_cmd', {
          conversationId, id, afterSeq: journal.cursor, waitMs: journal.cursor ? 2500 : 0,
        });
        if (cancelled || serial !== generation.current) return;
        if (policy !== null && policy !== next.privacyRevision) { setPage(null); setProjection(new SubagentJournalProjection()); setRevision(value => value + 1); return; }
        policy = next.privacyRevision;
        journal.append(next.journal?.events ?? []);
        setPage(next); setLoading(false); setError(''); render(value => value + 1);
        onStatus(id, next.status);
        if (next.journal?.historyTruncated) setTruncated(true);
        if (next.journal?.hasMore || !['completed', 'failed', 'cancelled', 'orphaned', 'superseded', 'timed_out'].includes(next.status)) {
          timer = setTimeout(() => void read(), next.journal?.hasMore ? 0 : 200);
        }
      } catch (cause) {
        if (!cancelled && serial === generation.current) { setError(String(cause)); setLoading(false); }
      }
    }
    void read();
    return () => { cancelled = true; clearTimeout(timer); };
  }, [conversationId, id, revision, onStatus]);
  const entries = projection.state.traceEvents;
  useEffect(() => {
    if (follow.current && scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [page, entries]);

  async function control(action: 'input' | 'cancel') {
    if (!page?.agentId || busy) return;
    const serial = generation.current;
    setBusy(true); setError('');
    try {
      await invoke('control_subagent_workspace_cmd', { conversationId, agentId: page.agentId, action, input: action === 'input' ? draft : null });
      if (serial !== generation.current) return;
      if (action === 'input') { setDraft(''); drafts.delete(draftKey); }
      else setPage(current => current ? { ...current, status: 'cancelling', canControl: false } : current);
      setRevision(value => value + 1);
    } catch (cause) { if (serial === generation.current) setError(String(cause)); }
    finally { setBusy(false); }
  }
  function resize(event: PointerEvent<HTMLDivElement>) {
    event.preventDefault();
    const start = event.clientX; const original = width;
    const target = event.currentTarget;
    target.setPointerCapture(event.pointerId);
    const move = (next: globalThis.PointerEvent) => setWidth(Math.max(380, Math.min(900, original + start - next.clientX)));
    const stop = () => { target.removeEventListener('pointermove', move); target.removeEventListener('lostpointercapture', stop); };
    target.addEventListener('pointermove', move); target.addEventListener('lostpointercapture', stop);
  }
  const status = page?.status ?? subtask.status;
  const statusLabel = ({ completed: t('chat.taskRunCompleted'), running: t('chat.taskRunRunning'), queued: t('chat.taskRunQueued'),
    failed: t('chat.taskRunFailed'), cancelled: t('chat.taskRunCancelled'), cancelling: t('chat.subagentStopping'),
    orphaned: t('chat.subagentDisconnected') } as Record<string, string>)[status] ?? status;
  const legacyInput = subagentRecord(page?.legacyRun?.input);
  const legacyOutputOuter = subagentRecord(page?.legacyRun?.output);
  const legacyOutput = subagentRecord(legacyOutputOuter.run ?? legacyOutputOuter);
  const task = projection.task || (typeof legacyInput.task === 'string' ? legacyInput.task : '');
  const result = projection.result || (typeof legacyOutput.result === 'string' ? legacyOutput.result : '');
  return (
    <aside data-testid="subagent-workspace" aria-label={t('chat.subagentWorkspace')} className="relative z-30 flex h-full shrink-0 flex-col border-l border-border bg-surface-1 text-text-primary max-lg:absolute max-lg:inset-0 max-lg:!w-full" style={{ width, maxWidth: '100%' }}
      onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); onClose(); } }}>
      <div role="separator" tabIndex={0} aria-orientation="vertical" aria-label={t('chat.subagentResize')} aria-valuenow={width} aria-valuemin={380} aria-valuemax={900}
        onPointerDown={resize} onKeyDown={event => { if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); setWidth(value => Math.max(380, Math.min(900, value + (event.key === 'ArrowLeft' ? 20 : -20)))); } }}
        className="absolute inset-y-0 -left-1 z-10 w-2 cursor-col-resize hover:bg-accent/20 focus:bg-accent/20 max-lg:hidden" />
      <header className="flex shrink-0 items-center gap-2 border-b border-border px-4 py-3">
        <Bot className="h-5 w-5 shrink-0 text-accent" />
        <div className="min-w-0 flex-1"><h2 className="truncate text-sm font-semibold">{compactTaskLabel(task) || t('chat.subagentWorkspace')}</h2><div className="flex items-center gap-1.5 text-xs text-text-secondary" role="status">{page?.canControl && <Loader2 className="h-3 w-3 animate-spin motion-reduce:animate-none" />}{projection.role && `${projection.role} · `}{statusLabel}</div></div>
        <button type="button" className="rounded p-2 hover:bg-surface-3" aria-label={t('chat.subagentRefresh')} onClick={() => setRevision(value => value + 1)}><RefreshCw size={16} /></button>
        <button ref={closeRef} type="button" className="rounded p-2 hover:bg-surface-3" aria-label={t('chat.subagentClose')} title={t('chat.subagentCloseHint')} onClick={onClose}><X size={18} /></button>
      </header>
      <div ref={scrollRef} className="min-h-0 flex-1 space-y-4 overflow-y-auto p-4 text-sm" onScroll={event => { const el = event.currentTarget; follow.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60; }}>
        <details open className="rounded-lg border border-border bg-surface-2 p-3"><summary className="cursor-pointer text-xs font-medium text-text-secondary">{t('chat.subagentTask')}</summary><p className="mt-2 whitespace-pre-wrap break-words">{task}</p></details>
        {(truncated || (page && !page.journal)) && <p className="text-xs text-warning">{t('chat.subagentHistoryPartial')}</p>}
        {loading && <div className="flex justify-center p-6"><Loader2 className="h-5 w-5 animate-spin" aria-label={t('chat.subagentLoading')} /></div>}
        {entries.length > visibleCount && <button className="w-full rounded border border-border py-2 text-xs hover:bg-surface-2" onClick={() => setVisibleCount(value => value + 100)}>{t('chat.subagentEarlier')}</button>}
        {entries.slice(-visibleCount).map(entry => (
          entry.kind === 'tool' ? <ToolCallCard key={entry.id} {...entry.toolCall} compact trace />
            : entry.kind === 'thinking' ? <details key={entry.id} className="rounded border border-border/60 bg-surface-2/50 p-3"><summary className="cursor-pointer text-xs text-text-secondary">{t('chat.subagentThinking')}</summary><div className="mt-2 whitespace-pre-wrap break-words text-xs leading-relaxed text-text-secondary">{entry.text}</div></details>
              : entry.kind === 'reply' ? <StreamingMarkdown key={entry.id} content={entry.text} isStreaming={false} reduceMotion />
                : <div key={entry.id} className={`whitespace-pre-wrap break-words rounded px-3 py-2 text-xs ${entry.code === 'inputQueued' ? 'bg-accent/10 text-text-primary' : entry.tone === 'error' ? 'text-danger' : 'bg-surface-2 text-text-secondary'}`}>
                    {entry.code === 'inputQueued' && <div className="mb-1 font-medium">{t('chat.subagentInputQueued')}</div>}
                    {entry.code === 'inputApplied' && <div className="mb-1 font-medium">{t('chat.subagentInputApplied')}</div>}
                    {entry.code === 'subagentApproval' && <div className="mb-1 font-medium">{t('chat.subagentApproval')}</div>}{entry.text}
                  </div>
        ))}
        {result && <section className="border-t border-border pt-4"><h3 className="mb-2 text-xs font-medium text-text-secondary">{t('chat.subagentResult')}</h3><StreamingMarkdown content={result} isStreaming={false} reduceMotion /></section>}
      </div>
      {error && <div role="alert" className="max-h-24 overflow-auto border-t border-border px-4 py-2 text-xs text-danger">{error}</div>}
      <form className="shrink-0 border-t border-border p-3" onSubmit={event => { event.preventDefault(); void control('input'); }}>
        <textarea aria-label={t('chat.subagentInstruction')} placeholder={t('chat.subagentInstruction')} disabled={!page?.canControl || busy} value={draft}
          onChange={event => { setDraft(event.target.value); drafts.set(draftKey, event.target.value); }} rows={2}
          className="w-full resize-none rounded-lg border border-border bg-surface-2 p-2 text-sm outline-none focus:border-accent disabled:opacity-50" />
        <div className="mt-2 flex items-center gap-2"><span className="min-w-0 flex-1 text-[11px] text-text-tertiary">{t('chat.subagentCloseHint')}</span>
          <button type="button" onClick={() => void control('cancel')} disabled={!page?.canControl || busy} className="flex items-center gap-1 rounded px-2 py-1.5 text-xs text-danger hover:bg-danger/10 disabled:opacity-40"><Square size={12} />{t('chat.subagentStop')}</button>
          <button type="submit" disabled={!page?.canControl || busy || !draft.trim()} className="flex items-center gap-1 rounded bg-accent px-3 py-1.5 text-xs text-white disabled:opacity-40"><Send size={12} />{t('chat.subagentSend')}</button>
        </div>
      </form>
    </aside>
  );
}
