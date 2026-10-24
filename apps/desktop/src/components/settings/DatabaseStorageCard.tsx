import { useEffect, useState } from 'react';
import { Database, RefreshCw } from 'lucide-react';
import { getDatabaseStorageStats, type DatabaseStorageStats } from '../../lib/api';
import { useTranslation } from '../../i18n';
import { Section, StatCard } from './SettingsSection';

function size(value: number | null): string {
  if (value === null) return '—';
  return value >= 1024 ** 3 ? `${(value / 1024 ** 3).toFixed(2)} GiB` : `${(value / 1024 ** 2).toFixed(1)} MiB`;
}

export function DatabaseStorageCard() {
  const { t } = useTranslation();
  const [stats, setStats] = useState<DatabaseStorageStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const [loading, setLoading] = useState(false);
  useEffect(() => {
    let active = true;
    setLoading(true);
    void getDatabaseStorageStats().then(value => { if (active) { setStats(value); setError(null); } })
      .catch(reason => { if (active) setError(String(reason)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [revision]);
  return <Section icon={<Database size={20} />} title={t('settings.storageDatabase')}>
    <div className="mb-3 flex items-center justify-between gap-3"><p className="text-xs leading-5 text-text-secondary">{t('settings.storageReuseHint')}</p><button type="button" disabled={loading} onClick={() => setRevision(value => value + 1)} aria-label={t('taskCenter.refresh')} className="rounded-lg p-2 text-text-secondary disabled:opacity-40"><RefreshCw size={15} className={loading ? 'animate-spin' : ''} /></button></div>
    {error && <p role="alert" className="text-xs text-danger">{error}</p>}
    {stats && <div className="grid grid-cols-3 gap-3"><StatCard label={t('settings.storageAllocated')} value={size(stats.allocatedBytes)} /><StatCard label={t('settings.storageReusable')} value={size(stats.reusableBytes)} /><StatCard label={t('settings.storageWal')} value={size(stats.walBytes)} /></div>}
  </Section>;
}
