import { ArrowUp, FolderOpen, Plus, X } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { toast } from 'sonner';
import { useTranslation } from '../../i18n';

export function WorkspaceRootsPicker({ roots, onChange }: { roots: string[]; onChange: (roots: string[]) => void }) {
  const { t } = useTranslation();
  const rows = roots.length ? roots : [''];
  const update = (index: number, value: string) => onChange(rows.map((root, i) => i === index ? value : root));
  const browse = async (index: number) => {
    try {
      const selected = await open({ directory: true, multiple: false, title: t('project.chooseFolder') });
      if (typeof selected === 'string') update(index, selected);
    } catch { toast.error(t('common.error')); }
  };
  return (
    <section className="space-y-3" data-testid="workspace-roots-picker">
      <div>
        <h3 className="text-sm font-medium text-text-primary">{t('project.workspaceFolders')}</h3>
        <p className="mt-1 text-xs leading-relaxed text-text-tertiary">{t('project.workspaceFoldersHint')}</p>
      </div>
      <div className="space-y-2">
        {rows.map((root, index) => (
          <div key={index} className="flex min-w-0 items-center gap-2 rounded-lg border border-border/80 bg-surface-0/60 px-3 py-2.5">
            <FolderOpen size={16} className="shrink-0 text-text-tertiary" />
            <div className="min-w-0 flex-1">
              <label htmlFor={`workspace-root-${index}`} className="text-[10px] font-medium text-text-tertiary">{index === 0 ? t('project.primaryFolder') : t('project.additionalFolder')}</label>
              <input id={`workspace-root-${index}`} value={root} onChange={event => update(index, event.target.value)} placeholder={t('project.folderPlaceholder')} className="block w-full min-w-0 bg-transparent py-0.5 text-sm text-text-primary outline-none placeholder:text-text-tertiary/70" />
            </div>
            <button type="button" onClick={() => void browse(index)} title={t('project.chooseFolder')} aria-label={t('project.chooseFolder')} className="rounded-md p-1.5 text-text-tertiary hover:bg-surface-2 hover:text-text-primary"><FolderOpen size={15} /></button>
            {index > 0 && <button type="button" onClick={() => onChange([root, ...rows.filter((_, i) => i !== index)])} title={t('project.makePrimary')} aria-label={t('project.makePrimary')} className="rounded-md p-1.5 text-text-tertiary hover:bg-surface-2"><ArrowUp size={14} /></button>}
            {root && <button type="button" onClick={() => onChange(rows.filter((_, i) => i !== index))} aria-label={t('common.remove')} className="rounded-md p-1 text-text-tertiary hover:text-danger"><X size={14} /></button>}
          </div>
        ))}
      </div>
      {roots.length < 16 && <button type="button" onClick={() => onChange([...rows, ''])} className="inline-flex items-center gap-1 text-xs text-text-tertiary hover:text-text-primary"><Plus size={13} />{t('project.addFolder')}</button>}
    </section>
  );
}
