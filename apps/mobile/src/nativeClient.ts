import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { cachedModelChoices, type ModelChoices } from '../../desktop/src/features/models/modelChoices';
import type { RemoteChatClient, PairedRemote, RemoteConnection, RemoteEnvelope } from '../../desktop/src/features/remote/remoteClient';

export interface NativeHost extends PairedRemote { id: string; endpoint: string }
export class NativeClient implements RemoteChatClient {
  readonly voiceAvailable = false;
  readonly models = cachedModelChoices(id => this.rpc<ModelChoices>('models.list', { connectionId: id }));
  state: RemoteConnection = { phase: 'connecting', endpoint: null };
  private listeners = new Set<(event: RemoteEnvelope) => void>();
  private connections = new Set<(state: RemoteConnection) => void>();
  private unlisten?: UnlistenFn;
  private closed = false;
  private readonly connectionId = crypto.randomUUID();
  constructor(readonly paired: NativeHost) {}
  subscribe(listener: (event: RemoteEnvelope) => void) { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; }
  subscribeConnection(listener: (state: RemoteConnection) => void) { this.connections.add(listener); listener(this.state); return () => { this.connections.delete(listener); }; }
  async start() {
    const unlisten = await listen<RemoteEnvelope & { hostId: string }>('mobile:remote', ({ payload }) => {
      if (this.closed || payload.hostId !== this.paired.id) return;
      if (payload.event === 'connection:state') {
        this.state = payload.payload;
        this.connections.forEach(listener => listener(this.state));
        if (this.state.phase === 'connected') this.listeners.forEach(listener => listener({ event: 'connection:resync', payload: {} }));
      } else this.listeners.forEach(listener => listener(payload));
    });
    if (this.closed) { unlisten(); return; }
    this.unlisten = unlisten;
    await invoke('mobile_remote', { hostId: this.paired.id, connectionId: this.connectionId, action: 'connect' });
    if (this.closed) await invoke('mobile_remote', { hostId: this.paired.id, connectionId: this.connectionId, action: 'close' });
  }
  rpc<T>(method: string, params?: unknown): Promise<T> {
    if (this.closed) return Promise.reject(new Error('This connection is closed.'));
    return invoke('mobile_remote', { hostId: this.paired.id, action: 'rpc', method, params });
  }
  audio(): Promise<void> { return Promise.reject(new Error('Voice capture is unavailable in this mobile version.')); }
  recoverAudio() { /* Voice capture is capability-gated. */ }
  close() { this.closed = true; this.unlisten?.(); void invoke('mobile_remote', { hostId: this.paired.id, connectionId: this.connectionId, action: 'close' }).catch(() => {}); }
}

export const local = <T>(action: string, input: unknown = {}) => invoke<T>('mobile_local', { action, input });
