import { RemoteChat } from '../../desktop/src/features/remote/RemoteChat';
import { RemotePreviewProvider } from '../../desktop/src/features/remote/RemotePreviewProvider';
import type { MobileChatPresentation } from '../../desktop/src/features/remote/MobileChatShell';
import type { NativeClient } from './nativeClient';
import { useState } from 'react';
import { RemoteScreen } from '../../desktop/src/features/remote/RemoteScreen';
import { useTranslation } from '../../desktop/src/i18n';

export default function ComputerChat({ client, presentation }: { client: NativeClient; presentation: MobileChatPresentation }) {
  const [screen, setScreen] = useState(false);
  const { t } = useTranslation();
  return <RemotePreviewProvider client={client}>
    {client.paired.manifest.capabilities?.includes('screen-control-v1') && <div className="px-3 py-2"><button className="rounded-lg border border-border px-3 py-2 text-xs" onClick={() => setScreen(value => !value)}>{t('remote.screenTitle')}</button>{screen && <RemoteScreen client={client} onClose={() => setScreen(false)}/>}</div>}
    <RemoteChat client={client} mobile={presentation} initialDraft="" onDraftConsumed={() => {}}/>
  </RemotePreviewProvider>;
}
