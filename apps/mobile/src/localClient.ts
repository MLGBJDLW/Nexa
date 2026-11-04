import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { AgentRunEvent } from '../../desktop/src/types/conversation';
import type { RemoteChatClient, PairedRemote, RemoteConnection, RemoteEnvelope } from '../../desktop/src/features/remote/remoteClient';
import { local, phoneSpeech } from './nativeClient';
import { cachedModelChoices, type ModelChoices } from '../../desktop/src/features/models/modelChoices';

export interface PhoneSettings { connectionId?: string | null; providerType: string; baseUrl: string | null; model: string }
interface Run { id: string; status: string; phase: string; errorMessage?: string }
export class LocalClient implements RemoteChatClient {
  readonly nativeSpeech = phoneSpeech;
  readonly voiceAvailable = false;
  readonly extendedModesAvailable = false;
  readonly imagesOnly = true;
  readonly approvalModeAvailable = true;
  readonly resumeAvailable = true;
  readonly paired: PairedRemote = { token: '', device: { id: 'phone', name: 'Phone' }, manifest: { serverId: 'local-phone', endpoints: [], reconnectGraceSeconds: 0 } };
  state: RemoteConnection = { phase: 'connected', endpoint: null };
  private listeners = new Set<(event: RemoteEnvelope) => void>();
  private unlisteners: UnlistenFn[] = [];
  private closed = false;
  private selected = '';
  private continuation: { checkpointId: string; requestId: string } | null = null;
  readonly models = cachedModelChoices(connectionId => local<ModelChoices>('models', { connectionId }));
  subscribe(listener: (event: RemoteEnvelope) => void) { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; }
  subscribeConnection(listener: (state: RemoteConnection) => void) { listener(this.state); return () => {}; }
  async start() {
    const subscriptions = await Promise.all([
      listen<{ conversationId: string; event: AgentRunEvent }>('mobile:run', ({ payload }) => this.emit({ event:'agent://run-event',payload:{ conversationId:payload.conversationId,runEvent:payload.event } })),
      listen('mobile:refresh', () => this.emit({ event:'connection:resync',payload:{} })),
      listen('mobile:connections', () => { this.models.invalidate(); this.emit({event:'connections:changed',payload:{}}); }),
    ]);
    if (this.closed) subscriptions.forEach(off => off()); else this.unlisteners = subscriptions;
    this.emit({ event:'connection:resync',payload:{} });
  }
  private emit(event: RemoteEnvelope) { if (!this.closed) this.listeners.forEach(listener => listener(event)); }
  async rpc<T>(method: string, raw?: unknown): Promise<T> { return await this.dispatch(method, raw as Record<string, any> ?? {}) as T; }
  private async dispatch(method: string, params: Record<string, any>): Promise<unknown> {
    if (this.closed) throw new Error('This phone session is closed.');
    switch (method) {
      case 'connections.list': return local('connections');
      case 'models.list': return this.models(params.connectionId);
      case 'chat.list': return local('list',{before:params.before});
      case 'chat.create': return local('create', { connectionId:params.connectionId, modelSelection:params.modelSelection });
      case 'chat.read':
        this.selected = params.conversationId;
        return local('messages',{conversationId:params.conversationId,beforeOrder:params.beforeOrder});
      case 'chat.message': return local(params.channel === 'thinking' ? 'thinking' : 'message',{conversationId:params.conversationId,messageId:params.messageId,offset:params.offset});
      case 'chat.start':
        return local('start',{conversationId:params.conversationId,message:params.message,requestId:params.idempotencyKey,options:params});
      case 'chat.stop': return local('stop');
      case 'chat.continue': {
        const state = await local<{ checkpoint?: { id:string }; run:Run | null }>('currentRun',{conversationId:params.conversationId});
        if (!state.checkpoint) throw new Error('There is no paused task to resume.');
        if (this.continuation?.checkpointId !== state.checkpoint.id) this.continuation = {checkpointId:state.checkpoint.id,requestId:crypto.randomUUID()};
        const result = await local('start',{conversationId:params.conversationId,message:'',...this.continuation,options:params});
        this.continuation = null; return result;
      }
      case 'chat.resume':
        this.selected = params.conversationId;
        return local('events',{conversationId:params.conversationId,runId:params.runId,afterSequence:params.afterSequence,durableHighWater:params.durableHighWater});
      case 'approvals.list': return local('approvals',{conversationId:this.selected});
      case 'approvals.respond': return local('approve',{requestId:params.requestId,allow:params.decision === 'allow_once'});
      case 'interactions.list': return [];
      default: throw new Error(`This operation is unavailable on the phone: ${method}`);
    }
  }
  audio(): Promise<void> { return Promise.reject(new Error('Voice capture is unavailable for phone tasks.')); }
  recoverAudio() {}
  close() { this.closed = true; this.unlisteners.forEach(off => off()); this.listeners.clear(); }
}
