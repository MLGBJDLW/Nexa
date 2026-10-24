import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router';
import { ArrowUpRight, RefreshCw, Workflow } from 'lucide-react';
import { useTranslation, type TranslationKey } from '../../i18n';
import { listAgentTaskRunSummaries } from '../../lib/api';
import { taskRunIsActive, taskRunIsSuspended } from '../../lib/streaming/runReconciliation';
import type { AgentTaskRunListItem } from '../../types/conversation';

function statusKey(item: AgentTaskRunListItem): TranslationKey {
  if (taskRunIsSuspended(item.run) || item.run.phase === 'approval') return 'taskCenter.waitingUser';
  if (taskRunIsActive(item.run)) return 'taskCenter.running';
  if (['failed', 'timed_out'].includes(item.run.status)) return 'taskCenter.failed';
  if (item.run.status === 'cancelled') return 'taskCenter.cancelled';
  return 'taskCenter.completed';
}

/** A bounded projection of existing tasks; opening Home never loads transcripts. */
export function TaskHome({ projectId }: { projectId?: string | null }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [tasks, setTasks] = useState<AgentTaskRunListItem[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    let alive = true;
    let pending = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    setTasks([]);
    setError(null);
    const load = async () => {
      if (pending || !alive || document.hidden) return;
      pending = true;
      clearTimeout(timer);
      try {
        const page = await listAgentTaskRunSummaries(12, null, null, projectId ?? null);
        if (!alive) return;
        setTasks(page?.items ?? []);
        setError(null);
        if (page?.items.some(item => taskRunIsActive(item.run))) timer = setTimeout(() => void load(), 15_000);
      } catch (reason) { if (alive) setError(String(reason)); }
      finally { pending = false; }
    };
    const refresh = () => void load();
    void load();
    window.addEventListener('focus', refresh);
    document.addEventListener('visibilitychange', refresh);
    return () => { alive = false; clearTimeout(timer); window.removeEventListener('focus', refresh); document.removeEventListener('visibilitychange', refresh); };
  }, [projectId, revision]);
  const groups = new Map<string, AgentTaskRunListItem[]>();
  for (const item of tasks) {
    const key = item.projectId ?? '';
    const group = groups.get(key) ?? [];
    group.push(item);
    groups.set(key, group);
  }
  return <section aria-label={t('taskCenter.title')} data-testid="task-home" className="mx-4 mb-5 min-w-0 rounded-xl border border-border/60 bg-surface-1/75 p-4 text-left">
    <header className="mb-3 flex flex-wrap items-center justify-between gap-2">
      <button type="button" onClick={() => navigate('/tasks')} className="flex items-center gap-2 text-sm font-medium text-text-primary">{t('taskCenter.title')}<ArrowUpRight size={14} /></button>
      <div className="flex items-center gap-3 text-xs text-text-secondary">
        <button type="button" className="flex items-center gap-1 hover:text-accent" onClick={() => navigate('/workflows')}><Workflow size={14} />{t('workflows.templates')}</button>
        <button type="button" aria-label={t('taskCenter.refresh')} onClick={() => setRevision(value => value + 1)}><RefreshCw size={14} /></button>
      </div>
    </header>
    {error && <p role="alert" className="mb-2 text-xs text-danger">{error}</p>}
    <div className="max-h-60 space-y-3 overflow-y-auto">
      {[...groups.entries()].map(([id, items]) => <div key={id}>
        {!projectId && <h3 className="mb-1 truncate text-xs text-text-tertiary">{items[0].projectName || t('taskCenter.noProject')}</h3>}
        <div className="grid gap-1 sm:grid-cols-2">{items.map(item => <button key={item.run.id} type="button" onClick={() => navigate(`/chat/${encodeURIComponent(item.run.conversationId)}`)} className="group min-w-0 rounded-lg border border-transparent px-3 py-2 text-left hover:border-border hover:bg-surface-2 focus-visible:outline-accent">
          <span className="flex items-start justify-between gap-2"><span className="line-clamp-2 text-sm text-text-primary">{item.run.title || item.conversationTitle}</span><ArrowUpRight size={13} className="shrink-0 text-text-tertiary" /></span>
          <span className="mt-1 block truncate text-xs text-text-tertiary">{t(statusKey(item))}{item.run.model ? ` · ${item.run.model}` : ''}</span>
          {item.run.errorMessage && <span className="mt-1 line-clamp-1 text-xs text-danger">{item.run.errorMessage}</span>}
        </button>)}</div>
      </div>)}
      {!tasks.length && !error && <p className="text-xs text-text-tertiary">{t('taskCenter.emptyTitle')}</p>}
    </div>
  </section>;
}
