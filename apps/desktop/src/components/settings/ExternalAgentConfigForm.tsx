import { useEffect, useRef, useState } from 'react';
import { ExternalLink, FolderOpen, Terminal } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { getExternalAgentLaunch, probeExternalAgent, type ExternalAgentLaunch } from '../../lib/externalAgents';
import { runtimeAgentConfig } from '../../lib/runtimeAgentConfig';
import type { CopilotModelSummary } from '../../lib/api';
import type { ProviderPreset } from '../../lib/providerPresets';
import type { AgentConfig, SaveAgentConfigInput } from '../../types/conversation';
import { Button } from '../ui/Button';

export function ExternalAgentConfigForm({ preset, config, onSave, onCancel, isSaving, onDirtyChange }: {
  preset: ProviderPreset; config?: AgentConfig; onSave: (input: SaveAgentConfigInput, launch?: ExternalAgentLaunch) => Promise<void>;
  onCancel: () => void; isSaving: boolean; onDirtyChange: (dirty: boolean) => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(config?.name ?? preset.name);
  const [launch, setLaunch] = useState<ExternalAgentLaunch>({ executable: null, workingDirectory: '' });
  const [initialLaunch, setInitialLaunch] = useState<ExternalAgentLaunch | null>(null);
  const [model, setModel] = useState(config?.model ?? '');
  const [models, setModels] = useState<CopilotModelSummary[]>([]);
  const [verified, setVerified] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generation = useRef(0);
  useEffect(() => {
    const current = ++generation.current;
    void getExternalAgentLaunch(preset.provider).then(value => {
      if (generation.current !== current) return;
      setLaunch(value); setInitialLaunch(value);
    }).catch(cause => { if (generation.current === current) setError(String(cause)); })
      .finally(() => { if (generation.current === current) setLoading(false); });
    return () => { generation.current += 1; };
  }, [preset.provider]);
  const changeLaunch = (next: ExternalAgentLaunch) => {
    generation.current += 1;
    setLaunch(next); setVerified(false); setModels([]); setLoading(false); setError(null); onDirtyChange(true);
  };
  const probe = async () => {
    const current = ++generation.current;
    setLoading(true); setError(null); setVerified(false);
    try {
      const found = await probeExternalAgent(preset.provider, launch);
      if (generation.current !== current) return;
      setModels(found); setVerified(true);
      setModel(previous => found.some(item => item.id === previous) ? previous : found[0]?.id ?? '');
    } catch (cause) { if (generation.current === current) setError(String(cause)); }
    finally { if (generation.current === current) setLoading(false); }
  };
  const unchanged = config?.model === model && JSON.stringify(initialLaunch) === JSON.stringify(launch);
  const canSave = !!name.trim() && !!launch.workingDirectory.trim() && !!model && !loading && !saving && !isSaving
    && (unchanged || (verified && models.some(item => item.id === model)));
  const save = async () => {
    if (!canSave) return;
    setSaving(true); setError(null);
    try {
      await onSave(runtimeAgentConfig(preset.provider, name, model, config), unchanged ? undefined : launch);
    } catch (cause) { setError(String(cause)); }
    finally { setSaving(false); }
  };
  const inputClass = 'mt-1 w-full min-w-0 rounded-lg border border-border bg-surface-2 px-3 py-2 text-sm text-text-primary focus:border-accent focus:outline-none';
  return <div className="min-w-0 space-y-4" data-testid="external-agent-form">
    <div className="rounded-xl border border-border bg-surface-2/60 p-3">
      <div className="flex items-center gap-2 text-sm font-medium"><Terminal size={16} />{preset.name}</div>
      <code className="mt-2 block break-all text-xs text-text-secondary">{preset.command} {preset.args?.join(' ')}</code>
      <p className="mt-2 text-xs leading-5 text-text-secondary">{t('settings.externalAgentOwnership')}</p>
      {preset.docsUrl && <a className="mt-2 inline-flex items-center gap-1 text-xs text-accent" href={preset.docsUrl} target="_blank" rel="noreferrer">
        {t('settings.externalAgentSetup')}<ExternalLink size={12} />
      </a>}
    </div>
    <label className="block text-xs font-medium text-text-secondary">{t('settings.providerName')}
      <input className={inputClass} value={name} onChange={event => { setName(event.target.value); onDirtyChange(true); }} />
    </label>
    <label className="block text-xs font-medium text-text-secondary">{t('settings.externalAgentExecutable')}
      <input className={inputClass} value={launch.executable ?? ''} placeholder={preset.command} disabled={!initialLaunch}
        onChange={event => changeLaunch({ ...launch, executable: event.target.value || null })} />
    </label>
    <label className="block text-xs font-medium text-text-secondary"><span className="inline-flex items-center gap-1"><FolderOpen size={13} />{t('settings.externalAgentDirectory')}</span>
      <input className={inputClass} value={launch.workingDirectory} disabled={!initialLaunch}
        onChange={event => changeLaunch({ ...launch, workingDirectory: event.target.value })} />
    </label>
    <p className="text-xs leading-5 text-text-tertiary">{t('settings.externalAgentDirectoryHint')}</p>
    {models.length > 0 && <label className="block text-xs font-medium text-text-secondary">{t('settings.defaultModel')}
      <select className={inputClass} value={model} onChange={event => { setModel(event.target.value); onDirtyChange(true); }}>
        {models.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
      </select>
    </label>}
    {verified && <p role="status" className="text-xs text-success">{t('settings.externalAgentConnected')}</p>}
    {error && <p role="alert" className="break-words text-xs text-danger [overflow-wrap:anywhere]">{error}</p>}
    <div className="flex flex-wrap gap-2">
      <Button size="sm" variant="secondary" loading={loading} disabled={!launch.workingDirectory.trim() || saving} onClick={() => void probe()}>{t('settings.externalAgentProbe')}</Button>
      <Button size="sm" loading={saving || isSaving} disabled={!canSave} onClick={() => void save()}>{t('common.save')}</Button>
      <Button size="sm" variant="secondary" onClick={onCancel}>{t('common.cancel')}</Button>
    </div>
  </div>;
}
