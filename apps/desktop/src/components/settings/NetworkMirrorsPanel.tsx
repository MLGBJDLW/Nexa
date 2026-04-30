import type { AppConfig } from '../../types/conversation';
import { useTranslation } from '../../i18n';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';
import { CollapsiblePanel } from './SettingsSection';

interface NetworkMirrorsPanelProps {
  appConfig: AppConfig;
  loading: boolean;
  onChange: (config: AppConfig) => void;
  onMarkDirty: () => void;
  onSave: () => void;
}

export function NetworkMirrorsPanel({
  appConfig,
  loading,
  onChange,
  onMarkDirty,
  onSave,
}: NetworkMirrorsPanelProps) {
  const { t } = useTranslation();

  return (
    <CollapsiblePanel
      title={t('settings.networkMirrors')}
      description={t('settings.networkMirrorsDesc')}
    >
      <div className="space-y-3">
        <div className="space-y-2">
          <label className="text-sm font-medium text-text-primary">{t('settings.hfMirrorLabel')}</label>
          <Input
            value={appConfig.hfMirrorBaseUrl ?? ''}
            onChange={(e) => {
              onChange({ ...appConfig, hfMirrorBaseUrl: e.target.value });
              onMarkDirty();
            }}
            placeholder="https://hf-mirror.com"
          />
          <p className="text-xs text-text-tertiary">{t('settings.hfMirrorHint')}</p>
        </div>
        <div className="space-y-2">
          <label className="text-sm font-medium text-text-primary">{t('settings.ghproxyLabel')}</label>
          <Input
            value={appConfig.ghproxyBaseUrl ?? ''}
            onChange={(e) => {
              onChange({ ...appConfig, ghproxyBaseUrl: e.target.value });
              onMarkDirty();
            }}
            placeholder="https://mirror.ghproxy.com"
          />
          <p className="text-xs text-text-tertiary">{t('settings.ghproxyHint')}</p>
        </div>
        <div className="flex justify-end">
          <Button
            size="sm"
            onClick={onSave}
            disabled={loading}
          >
            {loading ? '...' : t('common.save')}
          </Button>
        </div>
      </div>
    </CollapsiblePanel>
  );
}
