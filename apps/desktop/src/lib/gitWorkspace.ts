import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useRef, useState } from 'react';

export interface GitWorkspaceStatus {
  sourceId: string;
  root: string;
  branch: string;
  oid: string;
  upstream: string | null;
  ahead: number;
  behind: number;
  files: Array<{ path: string; index: string; worktree: string }>;
  truncated: boolean;
}

export interface GitWorkspaceSnapshot {
  repos: GitWorkspaceStatus[];
  issues: Array<{ sourceId: string; root: string; message: string }>;
  checkedSources: number;
}

const EMPTY_SNAPSHOT: GitWorkspaceSnapshot = { repos: [], issues: [], checkedSources: 0 };

export function useGitWorkspace(conversationId: string | null | undefined, active: boolean, revision: string) {
  const [state, setState] = useState<{ id: string; snapshot: GitWorkspaceSnapshot; error: string | null } | null>(null);
  const pendingRef = useRef<Promise<unknown> | null>(null);
  const requestRef = useRef<() => void>(() => {});
  const [diffRevision, setDiffRevision] = useState(0);
  const refresh = useCallback(() => {
    setDiffRevision(value => value + 1);
    requestRef.current();
  }, []);
  useEffect(() => {
    if (!conversationId) return;
    let disposed = false;
    let pending = false;
    let dirty = false;
    let queued: ReturnType<typeof setTimeout> | null = null;
    const schedule = () => {
      dirty = true;
      if (disposed || pending || queued !== null || document.hidden) return;
      // Coalesce tool completions, focus, and source changes in the same burst.
      queued = setTimeout(() => { queued = null; void load(); }, 150);
    };
    const load = async () => {
      if (disposed || pending || document.hidden) return;
      pending = true;
      dirty = false;
      try {
        await pendingRef.current?.catch(() => {});
        if (disposed) return;
        const request = invoke<GitWorkspaceSnapshot | GitWorkspaceStatus[]>('conversation_git_status_cmd', { conversationId });
        pendingRef.current = request;
        const snapshot = await request;
        if (!disposed) setState(previous => {
          const next = Array.isArray(snapshot) ? { ...EMPTY_SNAPSHOT, repos: snapshot, checkedSources: snapshot.length } : snapshot ?? EMPTY_SNAPSHOT;
          // Preserve repo identities on an unchanged poll: open diff previews
          // must not refetch (and spawn more Git processes) every five seconds.
          const sameRepos = previous?.id === conversationId && JSON.stringify(previous.snapshot.repos) === JSON.stringify(next.repos);
          const stable = sameRepos ? { ...next, repos: previous.snapshot.repos } : next;
          return previous?.id === conversationId && !previous.error && JSON.stringify(previous.snapshot) === JSON.stringify(stable)
            ? previous : { id: conversationId, snapshot: stable, error: null };
        });
      } catch (error) {
        if (!disposed) setState(previous => ({ id: conversationId, snapshot: previous?.id === conversationId ? previous.snapshot : EMPTY_SNAPSHOT, error: String(error) }));
      } finally {
        pending = false;
        if (dirty && !disposed) schedule();
      }
    };
    requestRef.current = schedule;
    schedule();
    const timer = window.setInterval(schedule, active ? 5000 : 15000);
    const onFocus = () => { if (!document.hidden) schedule(); };
    window.addEventListener('focus', onFocus);
    document.addEventListener('visibilitychange', onFocus);
    return () => {
      disposed = true;
      requestRef.current = () => {};
      if (queued !== null) clearTimeout(queued);
      window.clearInterval(timer);
      window.removeEventListener('focus', onFocus);
      document.removeEventListener('visibilitychange', onFocus);
    };
  }, [conversationId, active]);
  useEffect(refresh, [revision, refresh]);
  const current = state?.id === conversationId ? state : null;
  return { ...(current?.snapshot ?? EMPTY_SNAPSHOT), error: current?.error ?? null, loaded: current !== null, diffRevision, refresh };
}

export const getGitDiff = (conversationId: string, sourceId: string, path: string, staged: boolean) =>
  invoke<string>('conversation_git_diff_cmd', { conversationId, sourceId, path, staged });
