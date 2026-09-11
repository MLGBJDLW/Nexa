import { encodeLiveAudio, type LiveEvent, type LiveTransport } from '../live/liveTransport';
import { RemoteClient, RemoteNetworkError } from './remoteClient';
export function remoteLiveTransport(client: RemoteClient): LiveTransport {
  return {
    audioWindow: 16,
    recoverAudio: () => client.recoverAudio(),
    connections: () => client.rpc('live.connections'),
    models: client.models,
    start: (request) => client.rpc('live.start', { request }),
    snapshot: (sessionId) => client.rpc('live.snapshot', { sessionId }),
    frame: async (sessionId, mimeType, data) => {
      if (client.state.phase === 'connected') {
        try {
          await client.rpc('live.frame', { sessionId, mimeType, data });
        } catch (error) {
          if (!(error instanceof RemoteNetworkError)) throw error;
        }
      }
    },
    audio: (sessionId, data) => client.audio(sessionId, encodeLiveAudio(data)),
    stop: (sessionId) => client.rpc('live.stop', { sessionId }),
    summarize: (sessionId, connectionId, modelSelection) =>
      client.rpc('live.summarize', { sessionId, connectionId, modelSelection }),
    list: () => client.rpc('live.list'),
    load: (sessionId) => client.rpc('live.load', { sessionId }),
    subscribe: async (listener) =>
      client.subscribe((event) => {
        if (event.event === 'live:event') listener(event.payload as LiveEvent);
      }),
    connection: (listener) =>
      client.subscribeConnection((state) =>
        listener(
          state.phase === 'connected'
            ? 'connected'
            : state.phase === 'revoked' || state.phase === 'closed'
              ? 'closed'
              : 'reconnecting',
        ),
      ),
    isTransientError: (error) => error instanceof RemoteNetworkError,
  };
}
