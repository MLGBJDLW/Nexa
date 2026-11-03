import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { listWorkflowAutomations } from './api';
import type { WorkflowAutomation } from '../types/workflows';

/** One projection for the entire chat surface, never a poll per sidebar row. */
export function useConversationSchedules(revision: string) {
  const [tasks, setTasks] = useState<WorkflowAutomation[]>([]);
  const pending = useRef(false);
  const requested = useRef(false);
  const mounted = useRef(false);
  const refresh = useCallback(async () => {
    requested.current = true;
    if (pending.current || document.visibilityState === 'hidden') return;
    pending.current = true;
    try {
      do {
        requested.current = false;
        try {
          const next = await listWorkflowAutomations();
          if (mounted.current && !requested.current && Array.isArray(next)) setTasks(next);
        } catch { /* Keep the last confirmed projection during a transient disconnect. */ }
      } while (mounted.current && requested.current && document.visibilityState !== 'hidden');
    }
    finally { pending.current = false; }
  }, []);
  useEffect(() => {
    mounted.current = true;
    void refresh();
    const timer = window.setInterval(() => void refresh(), 60_000);
    const changed = () => void refresh();
    window.addEventListener('focus', changed);
    window.addEventListener('nexa:schedules-changed', changed);
    document.addEventListener('visibilitychange', changed);
    return () => {
      mounted.current = false;
      window.clearInterval(timer);
      window.removeEventListener('focus', changed);
      window.removeEventListener('nexa:schedules-changed', changed);
      document.removeEventListener('visibilitychange', changed);
    };
  }, [refresh]);
  useEffect(() => { void refresh(); }, [refresh, revision]);
  const byConversation = useMemo(() => {
    const grouped = new Map<string, WorkflowAutomation[]>();
    for (const task of tasks) {
      const id = task.conversationId ?? task.scheduleConfig.executionPolicy.conversationId;
      if (!id || task.triggerKind !== 'schedule') continue;
      const group = grouped.get(id) ?? [];
      group.push(task);
      grouped.set(id, group);
    }
    for (const group of grouped.values()) group.sort((a, b) => Number(b.enabled) - Number(a.enabled)
      || (a.nextRunAt ?? '').localeCompare(b.nextRunAt ?? '') || a.name.localeCompare(b.name));
    return grouped;
  }, [tasks]);
  return { byConversation, refresh };
}
