import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { connectEventSubscriptions } from '../../lib/eventSubscriptions';

export interface AgentPreviewRequest { requestId: string; path: string; resourcePaths?: string[]; line: number | null; conversationId: string | null; callId: string }
export interface AgentPreviewReceipt { path: string; kind: string; displayMode: string; warning: string | null }

export function useAgentPreviewRequests(open: (request: AgentPreviewRequest) => Promise<AgentPreviewReceipt>, cancel: (requestId: string) => void) {
  const openRef = useRef(open); openRef.current = open;
  const cancelRef = useRef(cancel); cancelRef.current = cancel;
  useEffect(() => {
    let active = true;
    const handled = new Set<string>();
    const pending = new Set<string>();
    const acknowledge = (requestId: string, receipt: AgentPreviewReceipt | null, error: string | null) => invoke('acknowledge_preview_request_cmd', { result: { requestId, receipt, error } });
    const receive = async (request: AgentPreviewRequest) => {
      if (!active || handled.has(request.requestId)) return;
      handled.add(request.requestId); pending.add(request.requestId);
      // Bound the replay guard without evicting in-flight work.
      if (handled.size > 64) for (const id of handled) { if (!pending.has(id)) { handled.delete(id); break; } }
      try {
        const receipt = await openRef.current(request);
        if (active && pending.has(request.requestId)) await acknowledge(request.requestId, receipt, null);
      } catch (error) {
        if (active && pending.has(request.requestId)) await acknowledge(request.requestId, null, String(error)).catch(() => {});
      } finally { pending.delete(request.requestId); }
    };
    const connection = connectEventSubscriptions([
      valid => listen<AgentPreviewRequest>('preview:open', event => { if (valid()) void receive(event.payload); }),
      valid => listen<{ requestId: string }>('preview:cancel', event => {
        if (!valid()) return;
        pending.delete(event.payload.requestId); cancelRef.current(event.payload.requestId);
      }),
    ], state => {
      if (state.status === 'ready') void invoke<AgentPreviewRequest[]>('pending_preview_requests_cmd').then(requests => { if (active) for (const request of requests ?? []) void receive(request); }).catch(() => {});
    });
    return () => {
      active = false; connection.stop();
      for (const id of pending) { cancelRef.current(id); void acknowledge(id, null, 'The Nexa preview surface was closed.').catch(() => {}); }
      pending.clear();
    };
  }, []);
}
