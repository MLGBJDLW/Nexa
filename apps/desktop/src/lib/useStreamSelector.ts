import { useCallback, useSyncExternalStore } from 'react';
import { streamStore, type StreamState } from './streamStore';

/** Selectors return stable values owned by the store, not a freshly allocated envelope. */
export function useStreamSelector<T>(conversationId: string, selector: (state: StreamState | undefined) => T): T {
  const subscribe = useCallback((listener: () => void) => streamStore.subscribe(changedId => {
    if (changedId === conversationId) listener();
  }), [conversationId]);
  const snapshot = useCallback(() => streamStore.selectStream(conversationId, selector), [conversationId, selector]);
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

export function useStreamTool(conversationId: string, callId: string, occurrence?: import('./streaming/protocol').ToolCallEvent) {
  const subscribe = useCallback((listener: () => void) => streamStore.subscribe(changedId => {
    if (changedId === conversationId) listener();
  }), [conversationId]);
  const snapshot = useCallback(() => streamStore.selectTool(conversationId,callId,occurrence), [conversationId,callId,occurrence]);
  return useSyncExternalStore(subscribe,snapshot,snapshot);
}
