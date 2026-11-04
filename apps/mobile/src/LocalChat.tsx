import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowUpRight } from 'lucide-react';
import { useTranslation } from '../../desktop/src/i18n';
import { RemoteChat } from '../../desktop/src/features/remote/RemoteChat';
import { Logo } from '../../desktop/src/components/Logo';
import { FilePreviewContext } from '../../desktop/src/features/preview/filePreviewContext';
import type { MobileChatPresentation } from '../../desktop/src/features/remote/MobileChatShell';
import { LocalClient, type PhoneSettings } from './localClient';

/** Both targets use the same ordered chat/replay/approval presentation. */
export function LocalChat({ settings, presentation }: { settings: PhoneSettings | null; presentation: MobileChatPresentation }) {
  const { t } = useTranslation();
  const [error, setError] = useState('');
  const [example, setExample] = useState('');
  const identity = JSON.stringify(settings);
  const label = t('remote.mobilePhone');
  const client = useMemo(() => new LocalClient(settings, label), [identity, label]);
  useEffect(() => { void client.start(); return () => client.close(); }, [client]);
  const previews = useMemo(() => ({ remote: true,
    openFilePreview: (path: string) => { void invoke('mobile_files',{action:'share',relativePath:path}).catch(error=>setError(String(error))); },
    openWebLink: (url: string) => { try { if (/^https?:$/.test(new URL(url).protocol)) window.open(url,'_blank','noopener,noreferrer'); } catch {} },
  }), []);
  return <FilePreviewContext.Provider value={previews}>{error && <p role="alert" className="mobile-error">{error}</p>}<RemoteChat client={client} mobile={presentation} initialDraft={example} onDraftConsumed={() => setExample('')} emptyState={
    <div className="mobile-empty"><div className="mobile-brand"><Logo size={44}/></div><h2>{t('remote.mobilePhoneTitle')}</h2>{settings ? <button className="mobile-example" onClick={() => setExample(t('remote.mobileExample'))}>{t('remote.mobileExample')}<ArrowUpRight size={16}/></button> : <button className="mobile-example" onClick={presentation.onConfigure}>{t('remote.mobileConfigure')}<ArrowUpRight size={16}/></button>}</div>
  }/></FilePreviewContext.Provider>;
}
