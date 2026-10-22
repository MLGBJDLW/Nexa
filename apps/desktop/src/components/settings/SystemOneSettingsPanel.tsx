import { SettingsRow } from './SettingsRow';
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
        <SettingsRow label={t('settings.systemOneEnable')} htmlFor={`${id}-enabled`}>
          <input id={`${id}-enabled`} className="self-end" type="checkbox" checked={config.enabled} disabled={loading} onChange={event => update({ enabled: event.target.checked })} />
        </SettingsRow>
        <p className="text-xs leading-5 text-text-tertiary">{t('settings.systemOnePrivacy')}</p>
        <div className="space-y-1">
          <div className="space-y-1.5">
            <SettingsRow label={t('settings.systemOneProvider')} htmlFor={`${id}-provider`}>
              <NexaSelect aria-label={t('settings.systemOneProvider')} id={`${id}-provider`} value={config.provider} disabled={loading} className="w-full rounded-md border border-border bg-surface-1 px-3 py-2 text-sm text-text-primary" onChange={event => {
                const next = changeSystemOneProvider(config, event.target.value);
                onChange({ ...appConfig, systemOne: next }); onMarkDirty();
              }}>
                {!preset && <option value={config.provider}>{config.provider}</option>}
                {SYSTEM_ONE_PROVIDERS.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
              </NexaSelect>
            </SettingsRow>
          </div>
          <div className="space-y-1.5">
            <SettingsRow label={t('settings.model')} htmlFor={`${id}-model`}>
              <Input aria-label={t('settings.model')} id={`${id}-model`} value={config.model} list={`${id}-models`} disabled={loading} onChange={event => update({ model: event.target.value })} />
            </SettingsRow>
            <datalist id={`${id}-models`}>{preset?.models.map(model => <option key={model.id} value={model.id}>{model.name}</option>)}</datalist>
            {config.provider === 'siliconflow-cn' && config.model === 'diffusiongemma' && <p role="status" className="text-xs text-warning">{t('settings.retiredModelSelection', { model: config.model })}</p>}
          </div>
        </div>
        <div className="space-y-1.5">
          <SettingsRow label={t('settings.apiKey')}><div className="flex gap-2">
              <Input aria-label={t('settings.apiKey')} id={`${id}-key`} type={showKey ? 'text' : 'password'} value={config.apiKey} disabled={loading} autoComplete="off" onChange={event => update({ apiKey: event.target.value })} />
              <Button variant="ghost" iconOnly aria-label={t(showKey ? 'settings.systemOneHideKey' : 'settings.systemOneShowKey')} onClick={() => setShowKey(!showKey)} icon={showKey ? <EyeOff size={16} /> : <Eye size={16} />} />
            </div></SettingsRow>
        </div>
        {preset && <div className="space-y-1.5">
          <SettingsRow label={t('settings.systemOneEndpoint')} htmlFor={`${id}-url`}>
            <Input aria-label={t('settings.systemOneEndpoint')} id={`${id}-url`} value={config.baseUrl ?? preset.baseUrl} placeholder={preset.baseUrlPlaceholder ?? preset.baseUrl} disabled={loading} onChange={event => update({ baseUrl: event.target.value || null })} />
          </SettingsRow>
        </div>}
        <p className="text-xs leading-5 text-text-tertiary">{t('settings.systemOneAccess')}</p>
        <div data-settings-actions className="flex flex-wrap items-center justify-between gap-3">
          {preset && <a className="text-xs text-accent underline" href={preset.documentationRef} target="_blank" rel="noreferrer">{t('settings.systemOneDocs')}</a>}
          <Button icon={<Save size={14} />} disabled={loading || (config.enabled && !systemOneConfigured(config))} onClick={() => void onSave({ ...appConfig, systemOne: config })}>{t('common.save')}</Button>
        </div>
      </div>
    </CollapsiblePanel>
  </div>;
}
