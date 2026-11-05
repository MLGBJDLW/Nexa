import { useEffect, useRef, useState } from 'react';
import { Archive, Trash2 } from 'lucide-react';
import type { StorageService, ConversationStorageCursor, ConversationStoragePage } from '../../lib/api';
import { useTranslation } from '../../i18n';
import { Button } from '../ui/Button';
import { Modal } from '../ui/Modal';
import { ConfirmDialog } from '../ui/ConfirmDialog';

export function ConversationStorageManager({ service, onClose, onChanged }: { service: StorageService; onClose: () => void; onChanged: () => void }) {
  const { t } = useTranslation();
  const [scope, setScope] = useState('active');
  const [page, setPage] = useState<ConversationStoragePage | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [writing, setWriting] = useState(false);
  const [error, setError] = useState('');
  const [revision, setRevision] = useState(0);
  const [confirm, setConfirm] = useState<string[] | null>(null);
  const generation = useRef(0);
  useEffect(() => {
    const request = ++generation.current; setBusy(true); setPage(null); setSelected(new Set()); setError('');
    void service.page(scope).then(value => { if (generation.current === request) setPage(value); })
      .catch(error => { if (generation.current === request) setError(String(error)); }).finally(() => { if (generation.current === request) setBusy(false); });
    return () => { generation.current++; };
  }, [service, scope, revision]);
  const selectAll = async () => {
    const request = generation.current; setBusy(true); setError('');
    try {
      const ids = new Set<string>(); let cursor: ConversationStorageCursor | null = null;
      do {
        const result: ConversationStoragePage = await service.page(scope, cursor);
        if (generation.current !== request) return;
        result.entries.forEach(entry => ids.add(entry.id)); cursor = result.next;
      } while (cursor);
      setSelected(ids);
    } catch (error) { if (generation.current === request) setError(String(error)); }
    finally { if (generation.current === request) setBusy(false); }
  };
  const next = async () => {
    if (!page?.next) return; const request = generation.current; setBusy(true);
    try { const result = await service.page(scope, page.next); if (generation.current === request) setPage(result); }
    catch (error) { if (generation.current === request) setError(String(error)); }
    finally { if (generation.current === request) setBusy(false); }
  };
  const mutate = async (operation: 'archive' | 'remove', ids: string[]) => {
    setBusy(true); setWriting(true); setError('');
    try { await service[operation](ids); setConfirm(null); onChanged(); setRevision(value => value + 1); }
    catch (error) { setError(String(error)); } finally { setBusy(false); setWriting(false); }
  };
  return <><Modal open onClose={() => { if (!writing) onClose(); }} title={t('settings.storageManageChats')}>
    <div className="space-y-3" data-testid="conversation-storage-manager">
      <select aria-label={t('settings.storageChatScope')} value={scope} disabled={busy} onChange={event => setScope(event.target.value)} className="w-full rounded-lg border border-border bg-surface-2 px-3 py-2 text-sm">
        <option value="active">{t('settings.storageActiveChats')}</option><option value="archived">{t('chat.archivedConversations')}</option><option value="all">{t('settings.storageAllChats')}</option>
      </select>
      <p className="text-xs text-text-tertiary">{t('settings.storageArchiveHint')}</p>
      {error && <p role="alert" className="text-xs text-danger">{error}</p>}
      <div className="flex flex-wrap gap-2"><Button size="sm" disabled={busy} onClick={() => void selectAll()}>{t('common.selectAll')}</Button><Button size="sm" variant="ghost" disabled={busy || !selected.size} onClick={() => setSelected(new Set())}>{t('common.clear')}</Button><span role="status" className="self-center text-xs text-text-tertiary">{busy ? t('common.loading') : t('chat.selectedCount',{count:selected.size})}</span></div>
      <div className="max-h-72 overflow-auto rounded-lg border border-border">{page?.entries.map(entry => <label key={entry.id} className="flex cursor-pointer items-center gap-3 border-b border-border/50 p-3 text-sm last:border-0"><input type="checkbox" checked={selected.has(entry.id)} disabled={busy} onChange={event => { const checked = event.target.checked; setSelected(current => { const next = new Set(current); if (checked) next.add(entry.id); else next.delete(entry.id); return next; }); }}/><span className="min-w-0 flex-1 truncate">{entry.title || t('chat.newChat')}</span>{entry.archivedAt && <Archive size={14} aria-label={t('chat.archive')}/>}</label>)}</div>
      {page?.next && <Button size="sm" variant="ghost" disabled={busy} onClick={() => void next()}>{t('settings.storageNextPage')}</Button>}
      <div className="flex flex-wrap gap-2"><Button size="sm" disabled={busy || !selected.size || scope === 'archived'} icon={<Archive size={14}/>} onClick={() => void mutate('archive',[...selected])}>{t('chat.archive')}</Button><Button size="sm" variant="danger" disabled={busy || !selected.size} icon={<Trash2 size={14}/>} onClick={() => setConfirm([...selected])}>{t('chat.deleteSelected')}</Button></div>
    </div>
  </Modal><ConfirmDialog open={!!confirm} onClose={() => { if (!busy) setConfirm(null); }} onConfirm={() => { if (confirm) void mutate('remove',confirm); }} title={t('chat.deleteBatchConfirm')} message={t('chat.deleteBatchConfirmDesc',{count:confirm?.length ?? 0})} loading={busy}/></>;
}
