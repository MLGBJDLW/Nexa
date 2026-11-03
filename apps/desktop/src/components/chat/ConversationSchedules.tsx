import { CalendarClock, Pause, Play } from 'lucide-react';
import { useState } from 'react';
import { Link } from 'react-router';
import { toast } from 'sonner';
import { useTranslation } from '../../i18n';
import * as api from '../../lib/api';
import { formatUserError } from '../../lib/userError';
import type { WorkflowAutomation } from '../../types/workflows';

export function scheduleTime(task: WorkflowAutomation, locale: string) {
  if (!task.nextRunAt) return '—';
  const date = new Date(task.nextRunAt);
  if (!Number.isFinite(date.getTime())) return '—';
  return new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', timeZone: task.scheduleConfig.timezone }).format(date);
}

export function ConversationScheduleBadge({ tasks }: { tasks: readonly WorkflowAutomation[] }) {
  const { t, locale } = useTranslation();
  const active = tasks.some(task => task.enabled);
  const title = tasks.map(task => `${task.name} · ${task.enabled ? scheduleTime(task, locale) : t('chat.schedulePaused')} · ${task.scheduleConfig.timezone}`).join('\n');
  return <span data-testid="conversation-schedule-badge" title={title} role="img" aria-label={`${t('chat.scheduledTasks')}: ${title}`} className={`inline-flex shrink-0 items-center gap-0.5 ${active ? 'text-accent' : 'text-text-tertiary'}`}>
    <CalendarClock size={13} />{tasks.length > 1 && <span className="text-[10px] tabular-nums">{tasks.length}</span>}
  </span>;
}

export function ConversationSchedules({ tasks, onRefresh }: { tasks: readonly WorkflowAutomation[]; onRefresh?: () => void }) {
  const { t, locale } = useTranslation();
  const [pending, setPending] = useState<string | null>(null);
  const toggle = async (task: WorkflowAutomation) => {
    setPending(task.id);
    try {
      await api.setWorkflowAutomationEnabled(task.id, !task.enabled);
      onRefresh?.();
    } catch (error) { toast.error(formatUserError(t('chat.scheduleManage'), error)); }
    finally { setPending(null); }
  };
  return <section data-testid="conversation-schedules" className="mx-1 mt-2 border-t border-border/50 pt-2">
    <div className="mb-1 flex items-center justify-between text-[10px] font-semibold text-text-tertiary">
      <span>{t('chat.scheduledTasks')}</span>
      <Link to="/workflows?tab=automations" className="rounded px-1 py-1 text-accent hover:underline">{t('chat.scheduleManage')}</Link>
    </div>
    <ul className="max-h-44 space-y-2 overflow-y-auto">
      {tasks.map(task => <li key={task.id} className="flex items-center gap-2">
        <CalendarClock className={`h-3.5 w-3.5 shrink-0 ${task.enabled ? 'text-accent' : 'text-text-tertiary'}`} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-xs" title={task.name}>{task.name}</div>
          <div className="mt-0.5 text-[10px] text-text-tertiary">{task.enabled ? `${t('chat.scheduleNext')}: ${scheduleTime(task, locale)}` : t('chat.schedulePaused')} · {task.scheduleConfig.timezone}</div>
          {task.status === 'waiting_approval' && <div className="text-[10px] text-warning">{t('chat.attentionApproval')}</div>}
        </div>
        <button type="button" disabled={pending !== null} onClick={() => void toggle(task)} aria-label={`${t(task.enabled ? 'chat.schedulePause' : 'chat.scheduleResume')}: ${task.name}`} className="rounded p-2 text-text-secondary hover:bg-surface-2 disabled:opacity-40">
          {task.enabled ? <Pause size={13} /> : <Play size={13} />}
        </button>
      </li>)}
    </ul>
  </section>;
}
