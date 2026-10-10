import { useEffect, useRef, useState } from 'react';
import { ExternalLink, FolderOpen, Terminal } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { getExternalAgentLaunch, inspectExternalAgent, parseExternalLaunchFields, type ExternalAgentLaunch, type ExternalAgentConfigOption } from '../../lib/externalAgents';
import { runtimeAgentConfig } from '../../lib/runtimeAgentConfig';
import { listMcpServers, type CopilotModelSummary } from '../../lib/api';
import type { McpServer } from '../../types/extensions';
import type { ProviderPreset } from '../../lib/providerPresets';
import type { AgentConfig, SaveAgentConfigInput } from '../../types/conversation';
import { Button } from '../ui/Button';

const isNativeModel = (option: ExternalAgentConfigOption) => option.category === 'model' || (!option.category && option.id === 'model');
const isNativeReasoning = (option: ExternalAgentConfigOption) => option.category === 'thought_level'
  || (!option.category && ['reasoning_effort','effort','thought_level'].includes(option.id));
const isModelDependent = (option: ExternalAgentConfigOption) => option.category === 'model_config' || option.category === 'thought_level'
  || (!option.category && ['reasoning_effort', 'effort', 'thought_level'].includes(option.id));

export function ExternalAgentConfigForm({ preset, config, onSave, onCancel, isSaving, onDirtyChange }: {
  preset: ProviderPreset; config?: AgentConfig; onSave: (input: SaveAgentConfigInput, launch?: ExternalAgentLaunch) => Promise<void>;
  onCancel: () => void; isSaving: boolean; onDirtyChange: (dirty: boolean) => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(config?.name ?? preset.name);
  const [launch, setLaunch] = useState<ExternalAgentLaunch>({ executable: null, workingDirectory: '' });
  const [initialLaunch, setInitialLaunch] = useState<ExternalAgentLaunch | null>(null);
  const [argsText, setArgsText] = useState('');
  const [envText, setEnvText] = useState('');
  let launchFieldsValid = true;
  try { parseExternalLaunchFields(argsText, envText); } catch { launchFieldsValid = false; }
  const packageRunner = ['npx', 'uvx'].includes(preset.command ?? '') && !launch.executable;
  const [model, setModel] = useState(config?.model ?? '');
  const [models, setModels] = useState<CopilotModelSummary[]>([]);
  const [nativeOptions, setNativeOptions] = useState<ExternalAgentConfigOption[]>([]);
  const [nativeCommands, setNativeCommands] = useState<string[]>([]);
  const [connectors, setConnectors] = useState<McpServer[]>([]);
  const [verified, setVerified] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generation = useRef(0);
  useEffect(() => {
    let cancelled = false;
    void listMcpServers().then(servers => { if (!cancelled) setConnectors(servers.filter(server => server.enabled && !server.builtinId)); }).catch(() => {});
    return () => { cancelled = true; };
  }, []);
  useEffect(() => {
    const current = ++generation.current;
    void getExternalAgentLaunch(config?.id).then(value => {
      if (generation.current !== current) return;
      setLaunch(value); setInitialLaunch(value);
      setArgsText(value.args == null ? '' : JSON.stringify(value.args, null, 2));
      setEnvText(Object.keys(value.env ?? {}).length ? JSON.stringify(value.env, null, 2) : '');
    }).catch(cause => { if (generation.current === current) setError(String(cause)); })
      .finally(() => { if (generation.current === current) setLoading(false); });
    return () => { generation.current += 1; };
  }, [config?.id]);
  const changeLaunch = (next: ExternalAgentLaunch) => {
    generation.current += 1;
    setLaunch(next); setVerified(false); setModels([]); setNativeOptions([]); setNativeCommands([]); setLoading(false); setError(null); onDirtyChange(true);
  };
  const probe = async (selected = model, requestedLaunch = launch) => {
    if (!launchFieldsValid) return;
    const current = ++generation.current;
    setLoading(true); setError(null); setVerified(false);
    try {
      const found = await inspectExternalAgent(preset.provider, requestedLaunch, selected || undefined);
      if (generation.current !== current) return;
      setModels(found.models); setNativeOptions(found.configOptions); setNativeCommands(found.commands); setVerified(true);
      setLaunch({ ...requestedLaunch, configOptions: Object.fromEntries(found.configOptions
        .filter(option => Object.prototype.hasOwnProperty.call(requestedLaunch.configOptions ?? {}, option.id))
        .map(option => [option.id, option.currentValue])) });
      setModel(selected || found.models[0]?.id || '');
    } catch (cause) { if (generation.current === current) setError(String(cause)); }
    finally { if (generation.current === current) setLoading(false); }
  };
  const unchanged = config?.model === model && JSON.stringify(initialLaunch) === JSON.stringify(launch);
  const canSave = !!name.trim() && !!model && !loading && !saving && !isSaving
    && launchFieldsValid
    && (unchanged || (verified && models.some(item => item.id === model)));
  const save = async () => {
    if (!canSave) return;
    setSaving(true); setError(null);
    try {
      const input = runtimeAgentConfig(preset.provider, name, model, config);
      // Native preferences may change the available reasoning values. A saved
      // chat override must not silently override the newly selected preference.
      if (JSON.stringify(initialLaunch?.configOptions) !== JSON.stringify(launch.configOptions)) input.reasoningEffort = null;
      await onSave(input, unchanged ? undefined : { ...launch, configOptionsModel: model });
    } catch (cause) { setError(String(cause)); }
    finally { setSaving(false); }
  };
  const inputClass = 'mt-1 w-full min-w-0 rounded-lg border border-border bg-surface-2 px-3 py-2 text-sm text-text-primary focus:border-accent focus:outline-none';
  return <div className="min-w-0 space-y-4" data-testid="external-agent-form">
    <div className="rounded-xl border border-border bg-surface-2/60 p-3">
      <div className="flex items-center gap-2 text-sm font-medium"><Terminal size={16} />{preset.name}</div>
      <code className="mt-2 block break-all text-xs text-text-secondary">{preset.command} {preset.args?.join(' ')}</code>
      {preset.registryVersion && <span className="mt-1 block text-[10px] text-text-tertiary">ACP Registry · {preset.registryVersion}</span>}
      <p className="mt-2 text-xs leading-5 text-text-secondary">{t('settings.externalAgentOwnership')}</p>
      {packageRunner && <p className="mt-2 text-xs leading-5 text-text-secondary">{t('settings.externalAgentPackageHint')}</p>}
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
    <p className="text-xs leading-5 text-text-tertiary">{t('settings.externalAgentDirectoryHint')}</p>
    <details className="rounded-lg border border-border p-3" open={Boolean(initialLaunch?.workingDirectory) || undefined}>
      <summary className="cursor-pointer text-xs font-medium text-text-secondary">{t('settings.advanced')}</summary>
    <label className="mt-3 block text-xs font-medium text-text-secondary"><span className="inline-flex items-center gap-1"><FolderOpen size={13} />{t('settings.externalAgentDirectory')}</span>
      <input className={inputClass} value={launch.workingDirectory} disabled={!initialLaunch}
        onChange={event => changeLaunch({ ...launch, workingDirectory: event.target.value })} />
    </label>
    <label className="mt-3 block text-xs font-medium text-text-secondary">{t('settings.externalAgentArguments')}
      <textarea className={`${inputClass} font-mono text-xs`} rows={3} value={argsText} placeholder={JSON.stringify(preset.args ?? [])} disabled={!initialLaunch}
        onChange={event => {
          setArgsText(event.target.value);
          try { changeLaunch({ ...launch, ...parseExternalLaunchFields(event.target.value, envText) }); }
          catch { changeLaunch(launch); }
        }} />
    </label>
    <label className="mt-3 block text-xs font-medium text-text-secondary">{t('settings.externalAgentEnvironment')}
      <textarea className={`${inputClass} font-mono text-xs`} rows={3} value={envText} placeholder='{}' disabled={!initialLaunch} spellCheck={false}
        onChange={event => {
          setEnvText(event.target.value);
          try { changeLaunch({ ...launch, ...parseExternalLaunchFields(argsText, event.target.value) }); }
          catch { changeLaunch(launch); }
        }} />
    </label>
    {!launchFieldsValid && <p role="alert" className="mt-2 text-xs text-danger">{t('settings.externalAgentJsonHint')}</p>}
    </details>
    {models.length > 0 && <label className="block text-xs font-medium text-text-secondary">{t('settings.defaultModel')}
      <select className={inputClass} value={model} disabled={loading} onChange={event => {
        const selected = event.target.value;
        const dependent = new Set(nativeOptions.filter(isModelDependent).map(option => option.id));
        const requestedLaunch = { ...launch, configOptionsModel: selected, configOptions: Object.fromEntries(Object.entries(launch.configOptions ?? {}).filter(([id]) => !dependent.has(id))) };
        setModel(selected); setLaunch(requestedLaunch); onDirtyChange(true); void probe(selected, requestedLaunch);
      }}>
        {model && !models.some(item => item.id === model) && <option value={model} disabled>{model} · {t('settings.modelCredentialUnavailable')}</option>}
        {models.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
      </select>
    </label>}
    {nativeOptions.filter(option => !isNativeModel(option) && !isNativeReasoning(option)).map(option => <label key={option.id} className="block text-xs font-medium text-text-secondary">
      {option.name}
      <select className={inputClass} disabled={loading} value={launch.configOptions?.[option.id] ?? option.currentValue}
        onChange={event => {
          const changesCatalog = !isModelDependent(option);
          const dependent = new Set(nativeOptions.filter(isModelDependent).map(item => item.id));
          const preferences = Object.fromEntries(Object.entries(launch.configOptions ?? {}).filter(([id]) => !changesCatalog || !dependent.has(id)));
          const requestedLaunch = { ...launch, configOptionsModel: changesCatalog ? undefined : model, configOptions: { ...preferences, [option.id]: event.target.value } };
          setLaunch(requestedLaunch); onDirtyChange(true); void probe(changesCatalog ? '' : model, requestedLaunch);
        }}>
        {option.options.map(choice => <option key={choice.value} value={choice.value}>{choice.name}</option>)}
      </select>
    </label>)}
    {connectors.length > 0 && <fieldset className="space-y-2 rounded-lg border border-border p-3">
      <legend className="px-1 text-xs font-medium text-text-secondary">{t('settings.externalAgentMcp')}</legend>
      <p className="text-xs leading-5 text-text-tertiary">{t('settings.externalAgentMcpHint')}</p>
      {connectors.map(server => <label key={server.id} className="flex items-center gap-2 text-sm text-text-primary">
        <input type="checkbox" disabled={loading} checked={launch.mcpServerIds?.includes(server.id) ?? false} onChange={event => {
          const ids = new Set(launch.mcpServerIds ?? []);
          if (event.target.checked) ids.add(server.id); else ids.delete(server.id);
          changeLaunch({ ...launch, mcpServerIds: [...ids] });
        }} />{server.name}
      </label>)}
    </fieldset>}
    {nativeCommands.length > 0 && <p className="break-words text-xs text-text-tertiary">{nativeCommands.map(name => `/${name}`).join(' · ')}</p>}
    {verified && <p role="status" className="text-xs text-success">{t('settings.externalAgentConnected')}</p>}
    {error && <p role="alert" className="break-words text-xs text-danger [overflow-wrap:anywhere]">{error}</p>}
    <div className="flex flex-wrap gap-2">
      <Button size="sm" variant="secondary" loading={loading} disabled={!initialLaunch || saving || !launchFieldsValid} onClick={() => void probe()}>{t(packageRunner ? 'settings.externalAgentPackageProbe' : 'settings.externalAgentProbe')}</Button>
      <Button size="sm" loading={saving || isSaving} disabled={!canSave} onClick={() => void save()}>{t('common.save')}</Button>
      <Button size="sm" variant="secondary" onClick={onCancel}>{t('common.cancel')}</Button>
    </div>
  </div>;
}
