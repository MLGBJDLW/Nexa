import { useId, useState } from 'react';
import { Eye, EyeOff, Save } from 'lucide-react';
import { useTranslation } from '../../i18n';
import type { AppConfig, SystemOneConfig } from '../../types/conversation';
import { changeSystemOneProvider, DEFAULT_SYSTEM_ONE_CONFIG, SYSTEM_ONE_PROVIDERS, systemOneConfigured } from '../../lib/systemOneProviders';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';
import { CollapsiblePanel } from './SettingsSection';
import { NexaSelect } from '../ui/overlay';

interface Props {
  appConfig: AppConfig;
  loading: boolean;
  onChange: (config: AppConfig) => void;
  onMarkDirty: () => void;
  onSave: (config?: AppConfig) => void | Promise<void>;
}

export function SystemOneSettingsPanel({ appConfig, loading, onChange, onMarkDirty, onSave }: Props) {
  const { t } = useTranslation();
  const id = useId();
  const [showKey, setShowKey] = useState(false);
  const config = appConfig.systemOne ?? DEFAULT_SYSTEM_ONE_CONFIG;
  const preset = SYSTEM_ONE_PROVIDERS.find(item => item.id === config.provider);
  const update = (patch: Partial<SystemOneConfig>) => {
    onChange({ ...appConfig, systemOne: { ...config, ...patch } });
    onMarkDirty();
  };
  return <div data-provider-category="structured-decisions" data-testid="system-one-settings">
    <CollapsiblePanel title={t('settings.systemOneTitle')} description={t('settings.systemOneDescription')} defaultOpen={false} testId="system-one-settings">
      <div className="space-y-3">
        <label className="flex items-center gap-2 text-sm text-text-primary">
          <input type="checkbox" checked={config.enabled} disabled={loading} onChange={event => update({ enabled: event.target.checked })} />
          {t('settings.systemOneEnable')}
        </label>
        <p className="text-xs leading-5 text-text-tertiary">{t('settings.systemOnePrivacy')}</p>
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="space-y-1.5">
            <label htmlFor={`${id}-provider`} className="text-sm text-text-primary">{t('settings.systemOneProvider')}</label>
            <NexaSelect id={`${id}-provider`} value={config.provider} disabled={loading} className="w-full rounded-md border border-border bg-surface-1 px-3 py-2 text-sm text-text-primary" onChange={event => {
              const next = changeSystemOneProvider(config, event.target.value);
              onChange({ ...appConfig, systemOne: next }); onMarkDirty();
            }}>
              {!preset && <option value={config.provider}>{config.provider}</option>}
              {SYSTEM_ONE_PROVIDERS.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
            </NexaSelect>
          </div>
          <div className="space-y-1.5">
            <label htmlFor={`${id}-model`} className="text-sm text-text-primary">{t('settings.model')}</label>
            <Input id={`${id}-model`} value={config.model} list={`${id}-models`} disabled={loading} onChange={event => update({ model: event.target.value })} />
            <datalist id={`${id}-models`}>{preset?.models.map(model => <option key={model.id} value={model.id}>{model.name}</option>)}</datalist>
          </div>
        </div>
        <div className="space-y-1.5">
          <label htmlFor={`${id}-key`} className="text-sm text-text-primary">{t('settings.apiKey')}</label>
          <div className="flex gap-2">
            <Input id={`${id}-key`} type={showKey ? 'text' : 'password'} value={config.apiKey} disabled={loading} autoComplete="off" onChange={event => update({ apiKey: event.target.value })} />
            <Button variant="ghost" iconOnly aria-label={t(showKey ? 'settings.systemOneHideKey' : 'settings.systemOneShowKey')} onClick={() => setShowKey(!showKey)} icon={showKey ? <EyeOff size={16} /> : <Eye size={16} />} />
          </div>
        </div>
        {preset && <div className="space-y-1.5">
          <label htmlFor={`${id}-url`} className="text-sm text-text-primary">{t('settings.systemOneEndpoint')}</label>
          <Input id={`${id}-url`} value={config.baseUrl ?? preset.baseUrl} placeholder={preset.baseUrlPlaceholder ?? preset.baseUrl} disabled={loading} onChange={event => update({ baseUrl: event.target.value || null })} />
        </div>}
        <p className="text-xs leading-5 text-text-tertiary">{t('settings.systemOneAccess')}</p>
        <div className="flex flex-wrap items-center justify-between gap-3">
          {preset && <a className="text-xs text-accent underline" href={preset.documentationRef} target="_blank" rel="noreferrer">{t('settings.systemOneDocs')}</a>}
          <Button icon={<Save size={14} />} disabled={loading || (config.enabled && !systemOneConfigured(config))} onClick={() => void onSave({ ...appConfig, systemOne: config })}>{t('common.save')}</Button>
        </div>
      </div>
    </CollapsiblePanel>
  </div>;
}
