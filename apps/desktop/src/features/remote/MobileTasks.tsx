import { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronRight, RefreshCw } from 'lucide-react';
import { useTranslation, type TranslationKey } from '../../i18n';
import { MobileDialog } from './MobileChatShell';
import type { RemoteChatClient } from './remoteClient';

type Cursor = [string, string, string];
type Task = {id: string; conversationId: string; title?: string; status: string; updatedAt: string};
type Page = {items: Task[]; nextCursor: Cursor | null};
const statusKeys: Record<string, TranslationKey> = {
  running: 'taskCenter.running', queued: 'taskCenter.queued', completed: 'taskCenter.completed',
  failed: 'taskCenter.failed', cancelled: 'taskCenter.cancelled', paused: 'taskCenter.paused',
  cancelling: 'taskCenter.cancelling', waiting_approval: 'taskCenter.waitingApproval',
  waiting_input: 'taskCenter.waitingUser', awaiting_user_input: 'taskCenter.waitingUser', timed_out: 'taskCenter.timedOut',
};

export function MobileTasks({client, onOpen, onClose}: {client: RemoteChatClient; onOpen: (id: string) => void; onClose: () => void}) {
  const {t, locale} = useTranslation();
  const [page, setPage] = useState<Page>({items: [], nextCursor: null});
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState('');
  const revision = useRef(0);
  const load = useCallback(async (before: Cursor | null = null) => {
    const generation = ++revision.current;
    setBusy(true); setError('');
    try {
      const next = await client.rpc<Page>('chat.tasks', {before});
      if (revision.current === generation) setPage(current => {
        const merged = new Map((before ? current.items : []).map(item => [item.id, item]));
        for (const task of next.items) merged.set(task.id, task);
        return {items: [...merged.values()], nextCursor: next.nextCursor};
      });
    } catch (cause) { if (generation === revision.current) setError(String(cause)); }
    finally { if (generation === revision.current) setBusy(false); }
  }, [client]);
  useEffect(() => {
    void load();
    return () => { revision.current++; };
  }, [load]);
  return <MobileDialog title={t('taskCenter.title')} onClose={onClose}>
    <button className="mobile-drawer-new" disabled={busy} onClick={() => void load()}><RefreshCw size={16}/>{t('taskCenter.refresh')}</button>
    {error && <p role="alert" className="mobile-error">{error}</p>}
    {busy && <p role="status">{t('taskCenter.loading')}</p>}
    {!busy && !error && !page.items.length && <p>{t('taskCenter.emptyTitle')}</p>}
    {page.items.map(task => <button className="mobile-task-row" key={task.id} onClick={() => onOpen(task.conversationId)}>
      <span><strong>{task.title || t('remote.conversation')}</strong><small>{t(statusKeys[task.status] ?? 'taskCenter.unknown')} · {new Date(task.updatedAt).toLocaleString(locale)}</small></span><ChevronRight size={16}/>
    </button>)}
    {page.nextCursor && <button className="mobile-drawer-new" disabled={busy} onClick={() => void load(page.nextCursor)}>{t('taskCenter.loadMore')}</button>}
  </MobileDialog>;
}
