import { useEffect, useState } from 'react';
import { getConversationFileChanges, type TurnFileChangeSummary } from './api';

const inFlight = new Map<string, Promise<TurnFileChangeSummary[]>>();
function query(conversationId: string, turnIds: string[], key: string) {
  const pending = inFlight.get(key);
  if (pending) return pending;
  const request = getConversationFileChanges(conversationId, turnIds);
  inFlight.set(key, request);
  const retire = () => { if (inFlight.get(key) === request) inFlight.delete(key); };
  void request.then(retire, retire);
  return request;
}

export function useConversationFileChanges(conversationId: string | null | undefined, active: boolean, completedTools: string, turnIds: string[]) {
  const turnKey = JSON.stringify([...new Set(turnIds)].sort());
  const [state, setState] = useState<{ conversationId: string; summaries: Map<string, TurnFileChangeSummary> } | null>(null);
  const pendingChanges = state?.conversationId === conversationId && [...(state?.summaries.values() ?? [])].some(summary => summary.pending);
  useEffect(() => {
    if (!conversationId) return;
    const selectedTurns = JSON.parse(turnKey) as string[];
    if (!selectedTurns.length) return;
    const queryKey = `${conversationId}:${turnKey}`;
    let disposed = false;
    let pending = false;
    const refresh = async () => {
      if (pending) return;
      pending = true;
      try {
        // A completion-triggered refresh must start after a preceding read;
        // that read may have captured its snapshot before the final mutation.
        const preceding = inFlight.get(queryKey);
        if (preceding) {
          await preceding.catch(() => {});
          if (disposed) return;
        }
        const values = await query(conversationId, selectedTurns, queryKey);
        if (!disposed && Array.isArray(values)) setState(previous => {
          const old = previous?.conversationId === conversationId ? previous.summaries : new Map<string, TurnFileChangeSummary>();
          const next = new Map(values.map(summary => [summary.turnId, summary]));
          if (old.size === next.size && values.every(value => old.get(value.turnId)?.revision === value.revision)) return previous;
          return { conversationId, summaries: next };
        });
      } catch (error) { console.warn('Could not refresh recorded file changes', error); }
      finally { pending = false; }
    };
    void refresh();
    // Child workers can commit files while their parent tool is still running.
    // Only small summaries are queried; details stay in the native database.
    const timer = active || pendingChanges ? window.setInterval(() => void refresh(), 2000) : null;
    return () => { disposed = true; if (timer !== null) window.clearInterval(timer); };
  }, [conversationId, active, completedTools, pendingChanges, turnKey]);
  return turnIds.length && state && state.conversationId === conversationId ? state.summaries : new Map<string, TurnFileChangeSummary>();
}
