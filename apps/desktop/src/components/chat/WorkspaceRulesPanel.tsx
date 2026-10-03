import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from '../../i18n';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';

interface WorkspaceRules {
  revision: string;
  files: Array<{ path: string; scope: string; revision: string; content: string; truncated: boolean }>;
  diagnostics: string[];
}

export function WorkspaceRulesPanel({ projectId }: { projectId: string }) {
  const { t } = useTranslation();
  const [path, setPath] = useState('');
  const [rules, setRules] = useState<WorkspaceRules | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const sequence = useRef(0);
  const load = useCallback(async (target?: string) => {
    const request = ++sequence.current;
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<WorkspaceRules>('get_project_rules_cmd', { projectId, path: target?.trim() || null });
      if (request === sequence.current) setRules(result);
    } catch (cause) {
      if (request === sequence.current) setError(String(cause));
    } finally {
      if (request === sequence.current) setLoading(false);
    }
  }, [projectId]);
  useEffect(() => {
    setRules(null);
    setPath('');
    void load();
    return () => { sequence.current += 1; };
  }, [load]);
  return <section className="space-y-3 rounded-lg border border-border bg-surface-1 p-3" data-testid="workspace-rules-panel">
    <div>
      <h3 className="text-sm font-medium">{t('project.fileRules')}</h3>
      <p className="mt-1 text-xs leading-relaxed text-text-tertiary">{t('project.fileRulesHint')}</p>
    </div>
    <form className="flex min-w-0 gap-2" onSubmit={event => { event.preventDefault(); void load(path); }}>
      <Input aria-label={t('project.ruleTarget')} value={path} onChange={event => setPath(event.target.value)} placeholder={t('project.ruleTarget')} />
      <Button type="submit" size="sm" loading={loading} className="shrink-0 whitespace-nowrap">{t('project.ruleRefresh')}</Button>
    </form>
    {error && <p role="alert" className="break-words text-xs text-danger">{error}</p>}
    {rules?.diagnostics.map(diagnostic => <p key={diagnostic} role="alert" className="break-words text-xs text-warning">{diagnostic}</p>)}
    {rules && !rules.files.length && !rules.diagnostics.length && <p className="text-xs text-text-tertiary">{t('project.fileRulesEmpty')}</p>}
    {rules?.files.map(rule => <details key={rule.path} className="rounded border border-border p-2">
      <summary className="cursor-pointer break-all text-xs font-medium">{rule.path}</summary>
      <p className="mt-2 break-all text-xs text-text-tertiary">{t('project.ruleScope')}: {rule.scope}</p>
      <p className="text-xs text-text-tertiary" title={rule.revision}>{t('project.ruleRevision')}: {rule.revision.slice(0, 12)}</p>
      {rule.truncated && <p role="status" className="text-xs text-warning">{t('project.ruleTruncated')}</p>}
      <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words text-xs">{rule.content}</pre>
    </details>)}
  </section>;
}
