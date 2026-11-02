import { useEffect, useRef, useState } from 'react';
import { FolderOpen, FolderPlus } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { toast } from 'sonner';
import { useTranslation } from '../../i18n';
import { createWorkspaceFolder } from '../../lib/api';

/** Explicit folder creation is shared by project roots and external-agent fallbacks. */
export function WorkspaceFolderActions({ path, onChange, disabled = false }: {
  path: string; onChange: (path: string) => void; disabled?: boolean;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const latest = useRef({ path, onChange });
  latest.current = { path, onChange };
  const generation = useRef(0);
  useEffect(() => {
    generation.current += 1;
    setBusy(false);
    return () => { generation.current += 1; };
  }, [path]);
  const run = async (create: boolean) => {
    const request = ++generation.current;
    const current = () => request === generation.current && latest.current.path === path;
    setBusy(true);
    try {
      const selected = create
        ? await createWorkspaceFolder(path.trim())
        : await open({ directory: true, multiple: false, title: t('project.chooseFolder') });
      if (!current()) return;
      if (typeof selected === 'string') latest.current.onChange(selected);
      if (create) toast.success(t('project.folderCreated'));
    } catch (error) { if (current()) toast.error(String(error)); }
    finally { if (current()) setBusy(false); }
  };
  return <>
    <button type="button" disabled={disabled || busy} onClick={() => void run(false)}
      title={t('project.chooseFolder')} aria-label={t('project.chooseFolder')}
      className="rounded-md p-1.5 text-text-tertiary hover:bg-surface-2 hover:text-text-primary disabled:opacity-40"><FolderOpen size={15} /></button>
    <button type="button" disabled={disabled || busy || !path.trim()} onClick={() => void run(true)}
      title={t('project.createFolder')} aria-label={t('project.createFolder')}
      className="rounded-md p-1.5 text-text-tertiary hover:bg-surface-2 hover:text-text-primary disabled:opacity-40"><FolderPlus size={15} /></button>
  </>;
}
