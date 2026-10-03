import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation, type TranslationKey } from '../../i18n';
import { Button } from '../ui/Button';

interface ProjectHook {
  id: string; name: string; manifestHash: string; event: 'after_file_change' | 'before_complete';
  arguments: Record<string, unknown>; enabled: boolean;
}
interface HookSnapshot {
  hooks: ProjectHook[];
  runs: Array<{ id: string; event: string; status: string; detail: string; createdAt: string }>;
  catalog: { tools: Array<{ name: string; manifestHash: string; commandPreview: string | null; runnable: boolean; warnings: string[] }>; errors: Array<{ path: string; message: string }> };
}
const statusKeys: Record<string, TranslationKey> = { passed: 'project.hookPassed', failed: 'project.hookFailed', cancelled: 'project.hookCancelled', running: 'project.hookRunning' };

export function ProjectHooksPanel({ projectId }: { projectId: string }) {
  const { t } = useTranslation();
  const [snapshot, setSnapshot] = useState<HookSnapshot | null>(null);
  const [selected, setSelected] = useState('');
  const [event, setEvent] = useState<ProjectHook['event']>('before_complete');
  const [argumentsText, setArgumentsText] = useState('{}');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sequence = useRef(0);
  const load = useCallback(async () => {
    const request = ++sequence.current;
    try {
      const next = await invoke<HookSnapshot>('get_project_hooks_cmd', { projectId });
      if (request === sequence.current) { setSnapshot(next); setError(null); }
    } catch (cause) { if (request === sequence.current) setError(String(cause)); }
  }, [projectId]);
  useEffect(() => {
    setSnapshot(null); setSelected(''); setArgumentsText('{}'); void load();
    return () => { sequence.current += 1; };
  }, [load]);
  const tools = snapshot?.catalog.tools.filter(tool => tool.runnable) ?? [];
  const selectedTool = tools.find(tool => `${tool.name}:${tool.manifestHash}` === selected);
  const mutate = async (command: string, args: Record<string, unknown>) => {
    setBusy(true); setError(null);
    try { await invoke(command, { projectId, ...args }); await load(); }
    catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };
  const enable = async () => {
    if (!selectedTool) return;
    try {
      const args: unknown = JSON.parse(argumentsText);
      if (!args || Array.isArray(args) || typeof args !== 'object') throw new Error(t('project.hookArguments'));
      const existing = snapshot?.hooks.find(hook => hook.name === selectedTool.name && hook.event === event);
      await mutate('save_project_hook_cmd', { hook: { id: existing?.id ?? '', name: selectedTool.name, manifestHash: selectedTool.manifestHash, arguments: args, enabled: true, event } });
    } catch (cause) { setError(String(cause)); }
  };
  const selectClass = 'w-full rounded-md border border-border bg-surface-2 p-2 text-xs';
  return <section className="space-y-3" data-testid="project-hooks-panel">
    <p className="text-xs leading-relaxed text-text-secondary">{t('project.hookHint')}</p>
    <Button size="sm" variant="ghost" disabled={busy} onClick={() => void load()}>{t('project.hookRefresh')}</Button>
    {error && <p role="alert" className="break-words text-xs text-danger">{error}</p>}
    {snapshot?.catalog.errors.map(item => <p key={item.path} className="break-all text-xs text-warning">{item.path}: {item.message}</p>)}
    {!tools.length && <p className="text-xs text-text-tertiary">{t('project.hookEmpty')}</p>}
    {!!tools.length && <div className="space-y-2 rounded-lg border border-border p-3">
      <select aria-label={t('project.hookTool')} className={selectClass} value={selected} onChange={e => setSelected(e.target.value)}>
        <option value="">{t('project.hookChoose')}</option>
        {tools.map(tool => <option key={`${tool.name}:${tool.manifestHash}`} value={`${tool.name}:${tool.manifestHash}`}>{tool.name}</option>)}
      </select>
      <select aria-label={t('project.hookEvent')} className={selectClass} value={event} onChange={e => setEvent(e.target.value as ProjectHook['event'])}>
        <option value="before_complete">{t('project.hookBefore')}</option><option value="after_file_change">{t('project.hookAfter')}</option>
      </select>
      <label className="block text-xs text-text-secondary">{t('project.hookArguments')}
        <textarea className={`${selectClass} mt-1 font-mono`} rows={3} value={argumentsText} onChange={e => setArgumentsText(e.target.value)} />
      </label>
      {selectedTool && <><pre className="overflow-auto whitespace-pre-wrap break-all rounded bg-surface-2 p-2 text-xs">{selectedTool.commandPreview}</pre>
        {selectedTool.warnings.map(warning => <p key={warning} className="text-xs text-warning">{warning}</p>)}
        <p className="break-all text-[10px] text-text-tertiary">{t('project.ruleRevision')}: {selectedTool.manifestHash.slice(0, 12)}</p></>}
      <Button size="sm" disabled={!selectedTool || busy} loading={busy} onClick={() => void enable()}>{t('project.hookEnable')}</Button>
    </div>}
    {snapshot?.hooks.map(hook => <div key={hook.id} className="flex items-start gap-2 rounded-lg border border-border p-3">
      <div className="min-w-0 flex-1"><p className="break-all text-sm font-medium">{hook.name}</p>
        <p className="text-xs text-text-tertiary">{t(hook.event === 'before_complete' ? 'project.hookBefore' : 'project.hookAfter')} · {t(hook.enabled ? 'project.hookEnabled' : 'project.hookDisabled')}</p>
        <code className="break-all text-[10px] text-text-tertiary">{hook.manifestHash.slice(0, 12)}</code></div>
      <Button size="sm" variant="ghost" disabled={busy} onClick={() => void mutate('save_project_hook_cmd', { hook: { ...hook, enabled: !hook.enabled } })}>{t(hook.enabled ? 'project.hookDisable' : 'project.hookEnable')}</Button>
      <Button size="sm" variant="ghost" disabled={busy} onClick={() => void mutate('delete_project_hook_cmd', { id: hook.id })}>{t('common.delete')}</Button>
    </div>)}
    {!!snapshot?.runs.length && <div className="space-y-2"><h3 className="text-sm font-medium">{t('project.hookHistory')}</h3>
      {snapshot.runs.map(run => <details key={run.id} className="rounded border border-border p-2">
        <summary className="cursor-pointer text-xs">{t(statusKeys[run.status] ?? 'project.hookFailed')} · {run.createdAt}</summary>
        <pre className="mt-2 max-h-56 overflow-auto whitespace-pre-wrap break-words text-xs">{run.detail}</pre>
      </details>)}
    </div>}
  </section>;
}
