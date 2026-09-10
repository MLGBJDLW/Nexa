import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { listen } from '@tauri-apps/api/event';
import { streamStore } from './streamStore';
import { parseAgentFrontendEvent } from './streaming/runEventWire';
import type { AgentHeartbeatEvent, AgentTaskSnapshotEvent } from '../types/conversation';
import { connectEventSubscriptions, type EventConnectionState } from './eventSubscriptions';
import { progressEventSubscriptions } from './progressEventSubscriptions';

const StreamConnection = createContext<{ state: EventConnectionState; retry: () => void }>({ state: { status: 'connecting' }, retry: () => {} });
export const useStreamConnection = () => useContext(StreamConnection);

/**
 * Global Tauri event listener for agent streaming events.
 * Mount once at app root — never tears down, so streams survive page navigation.
 */
export function StreamProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<EventConnectionState>({ status: 'connecting' });
  const connection = useRef<ReturnType<typeof connectEventSubscriptions> | null>(null);
  const retry = useCallback(() => connection.current?.retry(), []);
  useEffect(() => {
    const current = connectEventSubscriptions([
      (isActive) => listen<unknown>('agent://run-event', (event) => {
        if (!isActive()) return;
        const data = parseAgentFrontendEvent(event.payload);
        if (!data) {
          console.error('[StreamProvider] rejected invalid Run Event payload');
          return;
        }
        try {
          streamStore.dispatch(data.conversationId, data);
        } catch (err) {
          console.error('[StreamProvider] dispatch error:', err);
        }
      }),
      (isActive) => listen<AgentTaskSnapshotEvent>('agent://task-snapshot', (event) => {
        if (!isActive()) return;
        if (event.payload?.type !== 'taskRunUpdated') return;
        streamStore.applyTaskSnapshot(event.payload);
      }),
      (isActive) => listen<AgentHeartbeatEvent>('agent://heartbeat', (event) => {
        if (!isActive()) return;
        const heartbeat = event.payload;
        if (
          !heartbeat
          || typeof heartbeat.conversationId !== 'string'
          || typeof heartbeat.runId !== 'string'
          || typeof heartbeat.turnId !== 'string'
        ) return;
        streamStore.recordHeartbeat(heartbeat.conversationId, heartbeat.runId, heartbeat.durableHighWater);
      }),
      ...progressEventSubscriptions(),
    ], setState);
    connection.current = current;
    return () => { current.stop(); connection.current = null; };
  }, []);

  return <StreamConnection.Provider value={{ state, retry }}>{children}</StreamConnection.Provider>;
}
