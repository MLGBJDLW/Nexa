import { useRef, useState } from 'react';
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
  const run = async (create: boolean) => {
    setBusy(true);
    try {
      const selected = create
        ? await createWorkspaceFolder(path.trim())
        : await open({ directory: true, multiple: false, title: t('project.chooseFolder') });
      if (typeof selected === 'string' && latest.current.path === path) latest.current.onChange(selected);
      if (create) toast.success(t('project.folderCreated'));
    } catch (error) { toast.error(String(error)); }
    finally { setBusy(false); }
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
