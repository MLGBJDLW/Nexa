import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from '../../i18n';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';

interface Worktree { id: string; conversationId: string; path: string; branch: string; startSha: string; status: string; snapshotSha: string | null; indexSnapshotSha: string | null; detail: string | null }
export function ChatWorktreePanel({ conversationId }: { conversationId: string }) {
  const { t } = useTranslation();
  const [record, setRecord] = useState<Worktree | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [startRef, setStartRef] = useState('HEAD');
  const [busy, setBusy] = useState(false);
  const [archiveConfirmed, setArchiveConfirmed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sequence = useRef(0);
  useEffect(() => {
    const current = ++sequence.current; setLoaded(false); setRecord(null); setError(null); setBusy(false); setArchiveConfirmed(false);
    void invoke<Worktree | null>('get_chat_worktree_cmd', { conversationId }).then(value => {
      if (current === sequence.current) { setRecord(value); setLoaded(true); }
    }).catch(cause => { if (current === sequence.current) setError(String(cause)); });
    return () => { sequence.current++; };
  }, [conversationId]);
  const run = async (action: string) => {
    const current = ++sequence.current; setBusy(true); setError(null);
    try {
      const value = await invoke<Worktree | null>('change_chat_worktree_cmd', { conversationId, action, startRef });
      window.dispatchEvent(new CustomEvent('nexa:workspace-changed', { detail: { conversationId } }));
      if (current === sequence.current) { setRecord(value); setLoaded(true); setArchiveConfirmed(false); }
    } catch (cause) {
      if (current === sequence.current) {
        setError(String(cause));
        try { const value = await invoke<Worktree | null>('get_chat_worktree_cmd', { conversationId }); if (current === sequence.current) setRecord(value); } catch { /* Preserve the actionable operation error. */ }
      }
    } finally { if (current === sequence.current) setBusy(false); }
  };
  return <div data-testid="chat-worktree-panel" className="space-y-3 text-sm">
    <p className="text-xs leading-5 text-text-secondary">{t('chat.worktreeHelp')}</p>
    {error && <p role="alert" className="break-words text-xs text-danger">{error}</p>}
    {!loaded && !error && <p>{t('common.loading')}</p>}
    {loaded && !record && <><label className="block space-y-1 text-xs">{t('chat.worktreeStart')}<Input aria-label={t('chat.worktreeStart')} value={startRef} onChange={event => setStartRef(event.target.value)} disabled={busy} /></label><Button disabled={busy} onClick={() => void run('create')}>{t('chat.worktreeCreate')}</Button></>}
    {record && <>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-2 rounded-lg border border-border p-3 text-xs">
        <dt>{t('chat.worktreeState')}</dt><dd className="break-all">{record.status}</dd>
        <dt>{t('chat.worktreePath')}</dt><dd className="break-all select-text">{record.path}</dd>
        <dt>{t('chat.worktreeBranch')}</dt><dd className="break-all select-text">{record.branch}</dd>
        <dt>{t('chat.worktreeStart')}</dt><dd className="break-all font-mono select-text">{record.startSha}</dd>
        <dt>{t('chat.worktreeOwner')}</dt><dd className="break-all select-text">{record.conversationId}</dd>
        {record.snapshotSha && <><dt>{t('chat.worktreeSnapshot')}</dt><dd className="break-all font-mono select-text">{record.snapshotSha}</dd></>}
      </dl>
      {record.detail && <p className="break-words text-xs text-warning">{record.detail}</p>}
      <div className="flex flex-wrap gap-2">
        {record.status === 'ready' && <Button disabled={busy} onClick={() => void invoke('show_in_file_explorer', { path: record.path }).catch(cause => setError(String(cause)))}>{t('chat.worktreeOpen')}</Button>}
        {record.status === 'archived' && <Button disabled={busy} onClick={() => void run('restore')}>{t('chat.worktreeRestore')}</Button>}
        <Button disabled={busy} variant="secondary" onClick={() => void run('recover')}>{t('chat.worktreeRecover')}</Button>
      </div>
      {record.status === 'ready' && <div className="space-y-2 border-t border-border pt-3">
        <label className="flex items-start gap-2 text-xs leading-5 text-text-secondary"><input type="checkbox" className="mt-1" disabled={busy} checked={archiveConfirmed} onChange={event => setArchiveConfirmed(event.target.checked)} />{t('chat.worktreeArchiveHelp')}</label>
        <Button variant="secondary" disabled={busy || !archiveConfirmed} onClick={() => void run('archive')}>{t('chat.worktreeArchive')}</Button>
      </div>}
    </>}
    {busy && <p role="status" className="text-xs text-text-secondary">{t('common.loading')}</p>}
  </div>;
}
