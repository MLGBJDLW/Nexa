import { useEffect, useState } from 'react';
import { ChevronRight, Database, RefreshCw } from 'lucide-react';
import { storageService, type StorageService } from '../../lib/api';
import { invalidateStorageUsage, readStorageUsage, storageUsageSnapshot } from '../../lib/storageUsageCache';
import { useTranslation, type TranslationKey } from '../../i18n';
import { Section, StatCard } from './SettingsSection';
import { Button } from '../ui/Button';
import { ConversationStorageManager } from './ConversationStorageManager';

function size(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 ** 2) return `${(value / 1024).toFixed(1)} KiB`;
  return value >= 1024 ** 3 ? `${(value / 1024 ** 3).toFixed(2)} GiB` : `${(value / 1024 ** 2).toFixed(1)} MiB`;
}
const categories: Record<string, { label: TranslationKey; color: string }> = {
  chats: { label: 'settings.storageChats', color: '#818cf8' }, knowledge: { label: 'settings.storageKnowledge', color: '#38bdf8' },
  models: { label: 'settings.storageModels', color: '#c084fc' }, files: { label: 'settings.storageFiles', color: '#2dd4bf' },
  runtime: { label: 'settings.storageRuntime', color: '#fbbf24' }, extensions: { label: 'settings.storageExtensions', color: '#f472b6' },
  databaseOther: { label: 'settings.storageDatabaseOther', color: '#94a3b8' },
  wal: { label: 'settings.storageWal', color: '#fb923c' },
};
const reusePattern = 'repeating-linear-gradient(135deg, transparent 0 2px, #334155 2px 4px)';

const cleanupHints: Record<string, TranslationKey> = {
  chats: 'settings.storageCleanupChats', knowledge: 'settings.storageCleanupKnowledge',
  models: 'settings.storageCleanupModels', cache: 'settings.storageCleanupCache', logs: 'settings.storageCleanupLogs',
  database: 'settings.storageCleanupDatabase', extensions: 'settings.storageCleanupExtensions', managed: 'settings.storageCleanupManaged',
};

export function StorageUsageCard({ service = storageService, mobile = false }: { service?: StorageService; mobile?: boolean }) {
  const { t } = useTranslation();
  const [stats, setStats] = useState(() => storageUsageSnapshot(service));
  const [error, setError] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const [loading, setLoading] = useState(false);
  const [manage, setManage] = useState(false);
  const [changingAccess, setChangingAccess] = useState(false);
  async function changeAccess(allowed: boolean) {
    if (!service.setReportAccess || changingAccess) return;
    setChangingAccess(true);
    try { await service.setReportAccess(allowed); refresh(); }
    catch (reason) { setError(String(reason)); }
    finally { setChangingAccess(false); }
  }
  const refresh = () => {
    invalidateStorageUsage(service);
    setRevision(value => value + 1);
  };
  useEffect(() => {
    let active = true; setLoading(true); setStats(storageUsageSnapshot(service)); setError(null);
    void readStorageUsage(service).then(value => { if (active) { setStats(value); setError(null); } })
      .catch(reason => { if (active) setError(String(reason)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [service, revision]);
  return <Section icon={<Database size={20} />} title={t('settings.storageDistribution')}>
    <div aria-busy={loading} data-testid="storage-usage" className="min-w-0 space-y-3">
      <div className="flex items-center justify-between gap-3"><p className="text-xs leading-5 text-text-secondary">{t('settings.storageUsageHint')}</p><button type="button" disabled={loading} onClick={refresh} aria-label={t('taskCenter.refresh')} className="rounded-lg p-2 text-text-secondary disabled:opacity-40"><RefreshCw size={15} className={loading ? 'animate-spin' : ''} /></button></div>
      {loading && !stats && <p role="status" className="text-xs text-text-secondary">{t('common.loading')}</p>}
      {error && <p role="alert" className="text-xs text-danger">{error}</p>}
      {stats && <>
        <p className="text-2xl font-semibold tabular-nums">{size(stats.totalBytes)}</p>
        <div className="flex h-3 overflow-hidden rounded-full bg-surface-3" role="img" aria-label={t('settings.storageDistribution')}>
          {stats.categories.filter(category => category.bytes > 0).map(category => (
            <span key={category.id} className="flex justify-end" title={`${t(categories[category.id]?.label ?? 'settings.storageFiles')} · ${size(category.bytes)}`}
              style={{ width: `${category.bytes / Math.max(1, stats.totalBytes) * 100}%`, backgroundColor: categories[category.id]?.color ?? '#94a3b8' }}>
              {category.id === 'databaseOther' && stats.database.reusableBytes > 0 && (
                <span style={{ width: `${Math.min(1, stats.database.reusableBytes / category.bytes) * 100}%`, backgroundImage: reusePattern }} />
              )}
            </span>
          ))}
        </div>
        {stats.database.reusableBytes > 0 && <div className="flex items-baseline gap-2 text-xs text-text-secondary">
          <span aria-hidden="true" className="h-2.5 w-2.5 shrink-0 rounded-sm" style={{ backgroundColor: categories.databaseOther.color, backgroundImage: reusePattern }} />
          <div><p>{t('settings.storageReusable', { size: size(stats.database.reusableBytes) })}</p>
            <p className="mt-0.5 text-text-tertiary">{t('settings.storageReuseHint')}</p></div>
        </div>}
        <div className="divide-y divide-border/50 text-xs">{stats.categories.filter(category => category.bytes > 0).map(category =>
          <details key={category.id} className="group">
            <summary className="flex min-w-0 cursor-pointer list-none items-center gap-2 rounded py-2 focus-visible:outline-accent">
              <ChevronRight size={13} className="shrink-0 transition-transform group-open:rotate-90"/>
              <span className="h-2 w-2 shrink-0 rounded-full" style={{ backgroundColor: categories[category.id]?.color ?? '#94a3b8' }}/>
              <span className="flex-1">{t(categories[category.id]?.label ?? 'settings.storageFiles')}</span>
              <span className="tabular-nums text-text-secondary">{size(category.bytes)}</span>
            </summary>
            <ul className="space-y-2 pb-2 ps-5">{category.details?.map(detail =>
              <li key={JSON.stringify([detail.path, detail.table])} className="rounded-md bg-surface-2 p-2">
                <div className="flex items-start justify-between gap-2"><code dir="auto" className="select-text break-all">{detail.path}</code><span className="shrink-0 tabular-nums">{size(detail.bytes)}</span></div>
                {detail.table && <code className="block break-all text-text-secondary">{detail.table}</code>}
                <p className="mt-1 text-text-secondary">{t(cleanupHints[detail.cleanup] ?? 'settings.storageCleanupManaged')}</p>
              </li>)}</ul>
          </details>)}</div>
        {service.setReportAccess && <label className="flex items-center gap-2 text-xs text-text-secondary">
          <input type="checkbox" checked={!!stats.agentAccess} disabled={changingAccess || loading}
            onChange={event => void changeAccess(event.target.checked)} />{t('settings.storageAgentAccess')}
        </label>}
        {stats.partial && <p role="status" className="text-xs text-warning">{t('settings.storagePartial')}</p>}
        <div className="grid grid-cols-2 gap-3"><StatCard label={t('settings.storageActiveChats')} value={stats.activeConversations}/><StatCard label={t('chat.archivedConversations')} value={stats.archivedConversations}/></div>
      </>}
      <Button size="sm" variant="secondary" onClick={() => setManage(true)}>{t('settings.storageManageChats')}</Button>
      {manage && <ConversationStorageManager mobile={mobile} service={service} onClose={() => setManage(false)} onChanged={refresh}/>}
    </div>
  </Section>;
}
