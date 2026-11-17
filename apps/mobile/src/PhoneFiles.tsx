import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowRight, File, Trash2 } from 'lucide-react';
import { useTranslation } from '../../desktop/src/i18n';
import { MobileDialog } from '../../desktop/src/features/remote/MobileChatShell';

export function PhoneFiles() {
  const { t } = useTranslation();
  const [files, setFiles] = useState<string[] | null>(null);
  const [target, setTarget] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    let active = true;
    void invoke<string[]>('mobile_files', { action: 'list' }).then(paths => {
      if (active) setFiles(paths);
    }).catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; };
  }, []);

  const run = async (action: 'share' | 'delete', path: string) => {
    setBusy(true);
    setError('');
    try {
      await invoke('mobile_files', { action, relativePath: path });
      if (action === 'delete') {
        setFiles(current => current?.filter(file => file !== path) ?? null);
        setTarget(null);
      }
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  const closeConfirmation = () => {
    if (!busy) { setTarget(null); setError(''); }
  };

  return <>
    <p className="mobile-section-hint">{t('remote.mobileFilesHint')}</p>
    {!target && error && <p className="mobile-error" role="alert">{error}</p>}
    {!error && !files && <p role="status">{t('common.loading')}</p>}
    {files?.length === 0 && <p role="status">{t('sources.emptyFolder')}</p>}
    <div className="mobile-file-list">{files?.map(path =>
      <div className="mobile-file-row" key={path}>
        <button disabled={busy} onClick={() => void run('share', path)}>
          <File size={18}/><span>{path}</span><ArrowRight size={16}/>
        </button>
        <button className="mobile-file-delete" disabled={busy} aria-label={`${t('common.delete')}: ${path}`}
          onClick={() => { setError(''); setTarget(path); }}><Trash2 size={18}/></button>
      </div>
    )}</div>
    <MobileDialog open={target !== null} title={t('common.delete')} onClose={closeConfirmation}>
      <p>{target}</p>
      <p className="mobile-section-hint">{t('remote.deletePhoneFileHint')}</p>
      {error && <p className="mobile-error" role="alert">{error}</p>}
      <div className="mobile-confirm-actions">
        <button disabled={busy} onClick={closeConfirmation}>{t('common.cancel')}</button>
        <button className="mobile-primary" disabled={busy || !target} onClick={() => { if (target) void run('delete', target); }}>{t('common.delete')}</button>
      </div>
    </MobileDialog>
  </>;
}
