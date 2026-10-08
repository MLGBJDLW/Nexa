import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router';
import { X } from 'lucide-react';
import * as api from '../../lib/api';
import type { WorkflowAutomation, WorkflowRunHistoryEntry, WorkflowRunSnapshot } from '../../types/workflows';

type Props = { automation: WorkflowAutomation; tr: (key: string, params?: Record<string, string | number>) => string; onClose: () => void };

export function WorkflowRunHistory({ automation, tr, onClose }: Props) {
  const navigate = useNavigate();
  const [runs, setRuns] = useState<WorkflowRunHistoryEntry[]>([]);
  const [snapshot, setSnapshot] = useState<WorkflowRunSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [more, setMore] = useState(false);
  const load = async (before?: string) => {
    setBusy(true); setError(null);
    try { const next = await api.listWorkflowRunHistory(automation.id, before); setRuns(current => before ? [...current, ...next] : next); setMore(next.length === 25); }
    catch (reason) { setError(String(reason)); } finally { setBusy(false); }
  };
  useEffect(() => { void load(); }, [automation.id]);
  const inspect = async (run: WorkflowRunHistoryEntry) => {
    setError(null); setSnapshot(null);
    try { const result = await api.getWorkflowRunSnapshot(run.id); if (result) setSnapshot(result); else setError(tr('authoringSnapshotUnavailable')); }
    catch (reason) { setError(String(reason)); }
  };
  return <div className="fixed inset-0 z-60 flex items-center justify-center bg-black/50 p-4 backdrop-blur-sm"><section role="dialog" aria-modal="true" aria-labelledby="workflow-history-title" className="max-h-[90vh] w-full max-w-3xl overflow-y-auto rounded-2xl border border-border bg-surface-1 p-5 shadow-2xl">
    <header className="mb-4 flex justify-between"><div><h2 id="workflow-history-title" className="font-semibold text-text-primary">{tr('authoringHistory')}</h2><p className="text-xs text-text-secondary">{automation.name}</p></div><button type="button" aria-label={tr('authoringClose')} onClick={onClose}><X size={18} /></button></header>
    <div className="space-y-2">{runs.map(run => <article key={run.id} className="rounded-xl border border-border p-3 text-xs"><div className="flex flex-wrap justify-between gap-2"><p className="text-text-primary">{new Date(run.createdAt).toLocaleString()} · rev {run.definitionRevision} · {run.status}</p><div className="flex gap-3"><button type="button" className="text-accent" onClick={() => void inspect(run)}>{tr('authoringSnapshot')}</button>{run.taskRunId && <button type="button" className="text-accent" onClick={() => navigate(`/tasks?runId=${encodeURIComponent(run.taskRunId!)}`)}>{tr('authoringViewRun')}</button>}{run.conversationId && <button type="button" className="text-accent" onClick={() => navigate(`/chat/${run.conversationId}`)}>{tr('authoringOpenChat')}</button>}</div></div>{run.summary && <p className="mt-2 text-text-secondary">{run.summary}</p>}</article>)}</div>
    {!busy && runs.length === 0 && <p className="text-sm text-text-tertiary">{tr('authoringNoRuns')}</p>}
    {more && <button type="button" className="mt-3 text-sm text-accent" disabled={busy} onClick={() => void load(runs[runs.length - 1]?.id)}>{tr('authoringMoreRuns')}</button>}
    {error && <p role="alert" className="mt-3 text-sm text-danger">{error}</p>}
    {snapshot && <pre className="mt-4 max-h-80 overflow-auto whitespace-pre-wrap rounded-lg bg-surface-0 p-3 text-xs leading-5 text-text-secondary">{snapshot.compiledPrompt}</pre>}
  </section></div>;
}
