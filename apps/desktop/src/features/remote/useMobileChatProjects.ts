import { useCallback, useEffect, useRef, useState } from 'react';
import type { RemoteChatClient } from './remoteClient';

export interface ChatProjectSnapshot {
  projects: { id: string; name: string }[];
  memberships: Record<string, string>;
}

export interface ChatProjectStore {
  list(): Promise<ChatProjectSnapshot>;
  create(projectId: string, name: string): Promise<ChatProjectSnapshot>;
  rename(projectId: string, name: string): Promise<ChatProjectSnapshot>;
  remove(projectId: string): Promise<ChatProjectSnapshot>;
  move(conversationId: string, projectId: string | null): Promise<ChatProjectSnapshot>;
  activeChats?(projectId: string): Promise<ChatSummary[]>;
}

const emptyProjects: ChatProjectSnapshot = { projects: [], memberships: {} };
interface ChatSummary { id: string; title: string; model: string }

export function useMobileChatProjects(
  store: ChatProjectStore | undefined, client: RemoteChatClient, drawerOpen: boolean,
  conversations: ChatSummary[], cursor: string | null,
) {
  const [snapshot, setSnapshot] = useState<ChatProjectSnapshot | null>(null);
  const [selectedId, select] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [historyError, setHistoryError] = useState('');
  const [readingHistory, setReadingHistory] = useState(false);
  const [older, setOlder] = useState<ChatSummary[]>([]);
  const queue = useRef(Promise.resolve());

  // A focus refresh and a drop can overlap. Apply snapshots in operation order
  // so an older read cannot put a moved chat back in its previous folder.
  const apply = useCallback((operation: () => Promise<ChatProjectSnapshot>) => {
    const result = queue.current.then(async () => {
      try {
        const next = await operation();
        setSnapshot(next);
        select(current => next.projects.some(project => project.id === current) ? current : null);
        setError('');
      } catch (failure) {
        setError(String(failure));
        throw failure;
      }
    });
    queue.current = result.catch(() => {});
    return result;
  }, []);

  const refresh = useCallback(() => {
    if (store) void apply(() => store.list()).catch(() => {});
  }, [apply, store]);
  useEffect(() => {
    if (!drawerOpen || !store) return;
    refresh();
    window.addEventListener('focus', refresh);
    return () => window.removeEventListener('focus', refresh);
  }, [drawerOpen, refresh, store]);

  useEffect(() => {
    setOlder([]);
    setHistoryError('');
    setReadingHistory(false);
    if (!drawerOpen || !snapshot || !selectedId || !cursor) return;
    const missing = new Set(Object.keys(snapshot.memberships).filter(id => snapshot.memberships[id] === selectedId));
    conversations.forEach(chat => missing.delete(chat.id));
    if (!missing.size) return;
    let cancelled = false;
    async function readMembers(projectId: string, before: string | null) {
      const visited = new Set<string>();
      setReadingHistory(true);
      if (store?.activeChats) {
        const active = await store.activeChats(projectId);
        if (!cancelled) setOlder(active.filter(chat => missing.has(chat.id)));
        return;
      }
      // Paired computers own their transcripts. Ask only for summaries,
      // through the paged history API supported by existing desktop versions.
      while (before && missing.size) {
        if (visited.has(before)) throw new Error('Conversation history did not advance its cursor.');
        visited.add(before);
        const page: { items: ChatSummary[]; nextCursor: string | null } = await client.rpc('chat.list', { before });
        if (cancelled) return;
        const matches = page.items.filter(chat => missing.delete(chat.id));
        if (matches.length) setOlder(current => [...current, ...matches]);
        before = page.nextCursor;
      }
    }
    void readMembers(selectedId, cursor)
      .catch(failure => { if (!cancelled) setHistoryError(String(failure)); })
      .finally(() => { if (!cancelled) setReadingHistory(false); });
    return () => { cancelled = true; };
  }, [client, conversations, cursor, drawerOpen, selectedId, snapshot, store]);

  const projectView = store && {
    ...(snapshot ?? emptyProjects), selectedId, select, loading: (!snapshot && !error) || readingHistory,
    error: error || historyError, retry: refresh,
    create: async (name: string) => {
      const id = crypto.randomUUID();
      await apply(() => store.create(id, name));
      select(id);
    },
    rename: (id: string, name: string) => apply(() => store.rename(id, name)),
    remove: (id: string) => apply(() => store.remove(id)),
    move: (conversationId: string, projectId: string | null) => apply(() => store.move(conversationId, projectId)),
  };
  const drawerConversations = store && !snapshot ? [] : selectedId ? [...conversations, ...older] : conversations;
  return { projectView, drawerConversations };
}
