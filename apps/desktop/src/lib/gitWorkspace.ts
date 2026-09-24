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

export function useGitWorkspace(conversationId: string | null | undefined, active: boolean, revision: string) {
  const [state, setState] = useState<{ id: string; repos: GitWorkspaceStatus[]; error: string | null } | null>(null);
  const [refreshToken, setRefreshToken] = useState(0);
  const pendingRef = useRef<Promise<unknown> | null>(null);
  const refresh = useCallback(() => setRefreshToken(value => value + 1), []);
  useEffect(() => {
    if (!conversationId) return;
    let disposed = false;
    let pending = false;
    const load = async () => {
      if (disposed || pending || document.hidden) return;
      pending = true;
      try {
        // A new conversation/revision waits for the old IPC rather than
        // multiplying Git processes, then issues its own fresh scoped read.
        await pendingRef.current?.catch(() => {});
        if (disposed) return;
        const request = invoke<GitWorkspaceStatus[]>('conversation_git_status_cmd', { conversationId });
        pendingRef.current = request;
        const repos = await request;
        if (!disposed) setState({ id: conversationId, repos: Array.isArray(repos) ? repos : [], error: null });
      } catch (error) {
        if (!disposed) setState(previous => ({ id: conversationId, repos: previous?.id === conversationId ? previous.repos : [], error: String(error) }));
      } finally { pending = false; }
    };
    void load();
    const timer = window.setInterval(() => { void load(); }, active ? 5000 : 15000);
    const onFocus = () => { void load(); };
    window.addEventListener('focus', onFocus);
    document.addEventListener('visibilitychange', onFocus);
    return () => {
      disposed = true;
      window.clearInterval(timer);
      window.removeEventListener('focus', onFocus);
      document.removeEventListener('visibilitychange', onFocus);
    };
  }, [conversationId, active, revision, refreshToken]);
  return { repos: state && state.id === conversationId ? state.repos : [], error: state && state.id === conversationId ? state.error : null, refresh };
}

export const getGitDiff = (conversationId: string, sourceId: string, path: string, staged: boolean) =>
  invoke<string>('conversation_git_diff_cmd', { conversationId, sourceId, path, staged });
