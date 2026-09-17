import { useEffect, useState } from 'react';
import { RefreshCw, Save } from 'lucide-react';
import * as api from '../../lib/api';
import type { AppConfig } from '../../types/conversation';
import { useTranslation } from '../../i18n';
import { Button } from '../ui/Button';
import { SettingsRow, settingsSelectClass } from './SettingsRow';

export function ShellSettingsPanel({ config, saving, onChange, onSave }: {
  config: AppConfig; saving: boolean; onChange: (config: AppConfig) => void; onSave: () => void;
}) {
  const { t } = useTranslation();
  const [refresh, setRefresh] = useState(0);
  const [discovery, setDiscovery] = useState<api.ShellDiscovery | null>(null);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setFailed(false);
    void api.discoverShellEnvironments().then(result => {
      if (cancelled) return;
      if (!result || !Array.isArray(result.profiles)) { setFailed(true); return; }
      setDiscovery(result);
    }).catch(() => { if (!cancelled) setFailed(true); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [refresh]);
  const selected = config.defaultShell ?? '';
  const missing = selected !== '' && !discovery?.profiles.some(p => p.id === selected);
  return <div data-testid="shell-settings" className="border-t border-border pt-2">
    <SettingsRow label={t('settings.shellEnvironment')} description={t('settings.shellEnvironmentDesc')} htmlFor="default-shell">
      <select id="default-shell" value={selected} disabled={loading || saving} className={settingsSelectClass}
        onChange={event => onChange({ ...config, defaultShell: event.target.value })}>
        <option value="">{t('settings.shellAutomatic')}</option>
        {(discovery?.profiles ?? []).map(profile => <option key={profile.id} value={profile.id}>{profile.label}</option>)}
        {missing && <option value={selected}>{selected} · {t('settings.shellUnavailable')}</option>}
      </select>
    </SettingsRow>
    {config.shellAccessMode === 'restricted' && <p className="mb-2 text-xs text-text-tertiary">{t('settings.shellRestrictedHint')}</p>}
    {(failed || (discovery?.warnings.length ?? 0) > 0) && <p role="status" className="mb-2 text-xs text-warning">{t('settings.shellDiscoveryFailed')}</p>}
    {missing && !loading && <p role="status" className="mb-2 text-xs text-warning">{t('settings.shellMissingHint')}</p>}
    <div className="flex items-center justify-end gap-2">
      <Button size="sm" variant="ghost" icon={<RefreshCw size={13} />} disabled={loading || saving} onClick={() => setRefresh(value => value + 1)}>
        {loading ? t('common.loading') : t('settings.shellRefresh')}
      </Button>
      <Button size="sm" variant="secondary" icon={<Save size={13} />} disabled={loading || missing} loading={saving} onClick={onSave}>{t('common.save')}</Button>
    </div>
  </div>;
}
