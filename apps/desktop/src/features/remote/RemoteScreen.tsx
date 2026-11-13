import { useMemo } from 'react';
import { DeviceScreen } from './DeviceScreen';
import type { RemoteChatClient } from './remoteClient';
export function RemoteScreen({ client, onClose }: {client: RemoteChatClient; onClose?: () => void}) {
  const transport = useMemo(() => ({frame: () => client.rpc<import('./DeviceScreen').ScreenFrame>('screen.frame'), input: (input: Record<string, unknown>) => client.rpc('screen.input', {input}), stop: () => client.rpc('screen.stop')}), [client]);
  return <DeviceScreen transport={transport} onClose={onClose}/>;
}
