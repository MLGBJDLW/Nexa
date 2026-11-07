import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowUpRight } from 'lucide-react';
import { useTranslation } from '../../desktop/src/i18n';
import { RemoteChat } from '../../desktop/src/features/remote/RemoteChat';
import { Logo } from '../../desktop/src/components/Logo';
import { RemotePreviewProvider } from '../../desktop/src/features/remote/RemotePreviewProvider';
import type { MobileChatPresentation } from '../../desktop/src/features/remote/MobileChatShell';
import { LocalClient, type PhoneSettings } from './localClient';

const markReplyRead = (conversationId: string, runId: string) => invoke('mobile_local', {action: 'replyRead', input: {conversationId, runId}});

/** Both targets use the same ordered chat/replay/approval presentation. */
export function LocalChat({ settings, presentation }: { settings: PhoneSettings | null; presentation: MobileChatPresentation }) {
  const { t } = useTranslation();
  const [example, setExample] = useState('');
  const identity = JSON.stringify(settings);
  const label = t('remote.mobilePhone');
  const client = useMemo(() => new LocalClient(), [identity, label]);
  useEffect(() => { void client.start(); return () => client.close(); }, [client]);
  return <RemotePreviewProvider client={client}><RemoteChat client={client} mobile={{ ...presentation, onReplyRead: markReplyRead }} initialDraft={example} onDraftConsumed={() => setExample('')} emptyState={
    <div className="mobile-empty"><div className="mobile-brand"><Logo size={44}/></div><h2>{t('remote.mobilePhoneTitle')}</h2>{settings ? <button className="mobile-example" onClick={() => setExample(t('remote.mobileExample'))}>{t('remote.mobileExample')}<ArrowUpRight size={16}/></button> : <button className="mobile-example" onClick={presentation.onConfigure}>{t('remote.mobileConfigure')}<ArrowUpRight size={16}/></button>}</div>
  }/></RemotePreviewProvider>;
}
