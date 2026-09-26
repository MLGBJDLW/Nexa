import { useEffect, useState } from 'react';
import { Copy, RefreshCw } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { getGitDiff, type GitWorkspaceSnapshot } from '../../lib/gitWorkspace';

export function GitWorkspaceDetails({ conversationId, repos, issues, checkedSources, loaded, diffRevision, error, onRefresh }: GitWorkspaceSnapshot & {
  conversationId: string;
  loaded: boolean;
  diffRevision: number;
  error: string | null;
  onRefresh: () => void;
}) {
  const { t } = useTranslation();
  const [selection, setSelection] = useState<{ sourceId: string; path: string; staged: boolean } | null>(null);
  const [diff, setDiff] = useState<string | null>(null);
  const [diffError, setDiffError] = useState<string | null>(null);
  const [copyError, setCopyError] = useState(false);
  useEffect(() => {
    if (!selection) return;
    let disposed = false;
    setDiff(null);
    setDiffError(null);
    void getGitDiff(conversationId, selection.sourceId, selection.path, selection.staged)
      .then(value => { if (!disposed) setDiff(value); })
      .catch(cause => { if (!disposed) setDiffError(String(cause)); });
    return () => { disposed = true; };
  }, [conversationId, selection, repos, diffRevision]);
  return <section className="mx-1 mt-2 min-w-0 border-t border-border/50 pt-2" data-testid="git-workspace-details">
    <div className="flex items-center justify-between text-xs font-medium text-text-secondary">
      <span>Git</span>
      <button type="button" onClick={onRefresh} className="rounded p-1 hover:bg-surface-2" aria-label={t('chat.gitRefresh')}><RefreshCw size={13} /></button>
    </div>
    {error && <p role="status" className="break-words text-xs text-danger">{error}</p>}
    {issues.map(issue => <p key={issue.sourceId} role="status" className="break-words text-xs text-warning" title={issue.root}>{issue.root}: {issue.message}</p>)}
    {repos.length === 0 && !error && issues.length === 0 && <p role="status" className="py-2 text-xs text-text-tertiary">{!loaded ? t('common.loading') : checkedSources === 0 ? t('chat.gitNoSources') : t('chat.gitNoRepository')}</p>}
    {copyError && <p role="status" className="text-xs text-danger">{t('chat.gitCopyFailed')}</p>}
    <div className="max-h-64 overflow-auto">
      {repos.map(repo => <div key={repo.sourceId} className="mt-2 min-w-0">
        <div className="flex min-w-0 items-center gap-1 text-xs">
          <span className="min-w-0 flex-1 truncate font-medium" title={repo.root}>{repo.branch === '(detached)' ? repo.oid.slice(0, 8) : repo.branch}</span>
          {repo.upstream && <span className="shrink-0 text-text-tertiary" title={repo.upstream}>↑{repo.ahead} ↓{repo.behind}</span>}
          <button type="button" aria-label={t('chat.gitCopyBranch')} className="rounded p-1 hover:bg-surface-2" onClick={() => { setCopyError(false); void navigator.clipboard.writeText(repo.branch === '(detached)' ? repo.oid : repo.branch).catch(() => setCopyError(true)); }}><Copy size={12} /></button>
        </div>
        <p className="truncate text-[10px] text-text-tertiary" title={repo.root}>{repo.root}</p>
        {repo.files.length === 0 && <p className="py-2 text-xs text-text-tertiary">{t('chat.gitClean')}</p>}
        {repo.files.map(file => <div key={file.path} className="flex min-w-0 items-center gap-1 py-1 text-[11px]">
          <code className="shrink-0 text-accent">{file.index}{file.worktree}</code>
          <span className="min-w-0 flex-1 truncate" title={file.path}>{file.path}</span>
          {file.index !== '?' && file.index !== '.' && <button type="button" className="shrink-0 rounded px-1 text-accent hover:bg-surface-2" onClick={() => setSelection({ sourceId: repo.sourceId, path: file.path, staged: true })}>{t('chat.gitStaged')}</button>}
          {file.worktree !== '?' && file.worktree !== '.' && <button type="button" className="shrink-0 rounded px-1 text-accent hover:bg-surface-2" onClick={() => setSelection({ sourceId: repo.sourceId, path: file.path, staged: false })}>{t('chat.gitWorking')}</button>}
        </div>)}
        {repo.truncated && <p className="text-xs text-text-tertiary">{t('chat.gitTruncated')}</p>}
      </div>)}
    </div>
    {selection && <div className="mt-2 min-w-0 border-t border-border/50 pt-2">
      <div className="flex min-w-0 items-center gap-2 text-xs"><span className="min-w-0 flex-1 truncate" title={selection.path}>{selection.path}</span><button type="button" onClick={() => setSelection(null)} aria-label={t('common.close')}>×</button></div>
      <pre tabIndex={0} data-testid="git-diff-preview" className="mt-1 max-h-64 overflow-auto rounded bg-surface-0 p-2 text-[10px] text-text-secondary">{diffError ?? (diff === null ? t('common.loading') : diff || t('chat.gitNoDiff'))}</pre>
    </div>}
  </section>;
}
