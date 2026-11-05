import { NexaSelect } from '../../components/ui/overlay';
import { NexaCombobox } from '../../components/ui/overlay/Combobox';
import { useEffect, useState } from 'react';
import { Loader2, RefreshCw } from 'lucide-react';
import { useTranslation } from '../../i18n';
import type { ModelChoices, ModelChoicesLoader, TurnModelSelection } from './modelChoices';

const field = 'w-full min-w-0 rounded-xl border border-border bg-surface-1 px-3 py-2.5 text-sm text-text-primary outline-none focus:ring-2 focus:ring-accent/40 disabled:opacity-50';
export function ConnectionModelPicker({ connectionId, defaultModel, value, onChange, load, label, disabled = false }: {
  connectionId: string;
  defaultModel: string;
  value: TurnModelSelection | null;
  onChange: (value: TurnModelSelection | null) => void;
  load: ModelChoicesLoader;
  label: string;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const [catalog, setCatalog] = useState<ModelChoices | null>(null);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const [revision, setRevision] = useState(0);
  const [custom, setCustom] = useState(false);
  useEffect(() => {
    if (!connectionId) return;
    let cancelled = false;
    setLoading(true); setFailed(false);
    void load(connectionId, revision > 0).then(next => {
      if (!cancelled) { setCatalog(next); setFailed(Boolean(next.error)); }
    }).catch(() => { if (!cancelled) setFailed(true); }).finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [connectionId, load, revision]);
  const model = value?.model ?? defaultModel;
  const choices = catalog?.connectionId === connectionId ? catalog.models : [];
  const selected = choices.find(item => item.id === model);
  const changeModel = (id: string) => {
    const capability = choices.find(item => item.id === id)?.reasoning;
    onChange({ model: id, reasoningEnabled: capability ? true : null,
      reasoningEffort: capability?.defaultEffort === 'none' ? capability.effortLevels.find(level => level !== 'none') ?? null : capability?.defaultEffort ?? null, thinkingBudget: null });
  };
  const changeReasoning = (patch: Partial<TurnModelSelection>) => onChange({
    model, reasoningEnabled: selected?.reasoning ? true : null,
    reasoningEffort: value?.reasoningEffort ?? null, thinkingBudget: value?.thinkingBudget ?? null, ...patch,
  });
  return <div className="min-w-0 space-y-2" data-testid="connection-model-picker">
    <div className="flex items-end gap-2">
      <label className="min-w-0 flex-1 space-y-1.5 text-xs font-medium text-text-secondary">
        <span>{label}</span>
        {custom ? <input className={field} aria-label={label} value={model} maxLength={512} disabled={disabled}
          onChange={event => changeModel(event.target.value)} /> :
          <NexaCombobox className={field} ariaLabel={label} value={model} disabled={disabled || !connectionId} onValueChange={changeModel}
            searchPlaceholder={t('settings.modelSearchPlaceholder')} emptyLabel={t('settings.modelSearchNoResults')}
            options={[...(!selected ? [{value:model,label:model || t('settings.defaultModel')}] : []), ...choices.map(item => ({value:item.id,label:item.name,description:item.available === false ? t('settings.modelLifecycleRemoved') : item.id,disabled:item.available === false}))]}/>}
      </label>
      <button type="button" className="rounded-xl border border-border p-3 text-text-secondary disabled:opacity-40" aria-label={t('remote.refreshModels')} disabled={loading || disabled || !connectionId} onClick={() => setRevision(current => current + 1)}>
        {loading ? <Loader2 size={16} className="animate-spin" /> : <RefreshCw size={16} />}
      </button>
    </div>
    {selected?.available === false && <p role="status" className="text-xs text-warning">
      {t('settings.retiredModelSelection', { model })}
      {selected.replacementModelId && ` ${t('settings.suggestedModelReplacement', { model: selected.replacementModelId })}`}
    </p>}
    <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-text-tertiary">
      <span role="status">{loading ? t('remote.modelsLoading') : failed ? t('remote.modelsUnavailable') : ''}</span>
      <button type="button" disabled={disabled} className="text-accent" onClick={() => setCustom(current => !current)}>{custom ? t('remote.chooseModel') : t('remote.customModel')}</button>
    </div>
    {!!selected?.reasoning?.effortLevels.length && <label className="block space-y-1.5 text-xs text-text-secondary">
      <span>{t('settings.reasoningEffort')}</span>
      <NexaSelect className={field} disabled={disabled} aria-label={`${label} · ${t('settings.reasoningEffort')}`} value={value?.reasoningEffort === 'none' ? '' : value?.reasoningEffort ?? ''}
        onChange={event => changeReasoning({ reasoningEffort:event.target.value || null, thinkingBudget:null })}>
        <option value="">{t('settings.isDefault')}</option>
        {selected.reasoning.effortLevels.filter(effort => effort !== 'none').map(effort => <option key={effort} value={effort}>{effort}</option>)}
      </NexaSelect>
    </label>}
    {selected?.reasoning?.thinkingBudget && selected.reasoning.thinkingBudget.enabled !== false && <label className="block space-y-1.5 text-xs text-text-secondary">
      <span>{t('settings.thinkingBudget')}</span>
      <input className={field} type="number" disabled={disabled} min={Math.max(1, selected.reasoning.thinkingBudget.minTokens ?? 1)} max={selected.reasoning.thinkingBudget.maxTokens} value={value?.thinkingBudget ?? ''}
        onChange={event => changeReasoning({ thinkingBudget:event.target.value ? Number(event.target.value) : null, reasoningEffort:null })} />
    </label>}
  </div>;
}
