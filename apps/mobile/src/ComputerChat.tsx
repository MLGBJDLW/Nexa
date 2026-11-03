import { RemoteChat } from '../../desktop/src/features/remote/RemoteChat';
import { RemotePreviewProvider } from '../../desktop/src/features/remote/RemotePreviewProvider';
import type { MobileChatPresentation } from '../../desktop/src/features/remote/MobileChatShell';
import type { NativeClient } from './nativeClient';

export default function ComputerChat({ client, presentation }: { client: NativeClient; presentation: MobileChatPresentation }) {
  return <RemotePreviewProvider client={client}><RemoteChat client={client} mobile={presentation} initialDraft="" onDraftConsumed={() => {}}/></RemotePreviewProvider>;
}
