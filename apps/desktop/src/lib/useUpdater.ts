import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { useState, useEffect, useCallback } from 'react';

interface UpdateState {
  status: 'idle' | 'checking' | 'available' | 'downloading' | 'ready' | 'error' | 'up-to-date';
  version?: string;
  notes?: string;
  progress?: number;
  error?: string;
  errorCode?: string | number | null;
  errorDetail?: { stack?: string };
  errorStage?: 'check' | 'download' | 'install';
  lastCheckedAt?: string;
}

const UPDATE_CHECK_TIMEOUT_MS = 90_000;
const UPDATE_DOWNLOAD_TIMEOUT_MS = 600_000;

let sharedState: UpdateState = { status: 'idle' };
let sharedUpdate: Update | null = null;
let autoCheckStarted = false;
const listeners = new Set<(state: UpdateState) => void>();

function setSharedState(next: UpdateState | ((prev: UpdateState) => UpdateState)) {
  sharedState = typeof next === 'function'
    ? (next as (prev: UpdateState) => UpdateState)(sharedState)
    : next;
  for (const listener of listeners) {
    listener(sharedState);
  }
}

function extractError(e: unknown): { error: string; errorCode: string | number | null; errorDetail: { stack?: string } } {
  const errMsg = e instanceof Error ? e.message : String(e);
  const errCode = (e as { code?: string | number; status?: string | number } | null)?.code
    ?? (e as { code?: string | number; status?: string | number } | null)?.status
    ?? null;
  const errStack = e instanceof Error ? e.stack : undefined;
  return { error: errMsg, errorCode: errCode, errorDetail: { stack: errStack?.slice(0, 500) } };
}

export function useUpdater(checkOnMount = true) {
  const [state, setState] = useState<UpdateState>(sharedState);

  const checkForUpdate = useCallback(async () => {
    setSharedState({ status: 'checking' });
    try {
      const update = await check({ timeout: UPDATE_CHECK_TIMEOUT_MS });
      const lastCheckedAt = new Date().toISOString();
      if (update) {
        sharedUpdate = update;
        setSharedState({
          status: 'available',
          version: update.version,
          notes: update.body ?? undefined,
          lastCheckedAt,
        });
        return update;
      } else {
        sharedUpdate = null;
        setSharedState({ status: 'up-to-date', lastCheckedAt });
        return null;
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      // Graceful fallback: missing release manifest (404) → treat as up-to-date
      if (/\b404\b|Not Found/i.test(msg)) {
        sharedUpdate = null;
        setSharedState({ status: 'up-to-date', lastCheckedAt: new Date().toISOString() });
        return null;
      }
      setSharedState({ status: 'error', errorStage: 'check', lastCheckedAt: new Date().toISOString(), ...extractError(e) });
      return null;
    }
  }, []);

  const downloadAndInstall = useCallback(async () => {
    let update = sharedUpdate;
    if (!update) {
      try {
        update = await check({ timeout: UPDATE_CHECK_TIMEOUT_MS });
        if (update) sharedUpdate = update;
      } catch (e) {
        setSharedState({ status: 'error', errorStage: 'check', lastCheckedAt: new Date().toISOString(), ...extractError(e) });
        return;
      }
      if (!update) return;
    }

    setSharedState(prev => ({
      ...prev,
      status: 'downloading',
      progress: 0,
      error: undefined,
      errorCode: undefined,
      errorDetail: undefined,
      errorStage: undefined,
    }));

    let downloaded = 0;
    let contentLength = 0;

    try {
      await update.downloadAndInstall(
        (event) => {
          switch (event.event) {
            case 'Started':
              contentLength = event.data.contentLength ?? 0;
              break;
            case 'Progress':
              downloaded += event.data.chunkLength;
              if (contentLength > 0) {
                setSharedState(prev => ({
                  ...prev,
                  progress: Math.round((downloaded / contentLength) * 100),
                }));
              }
              break;
            case 'Finished':
              setSharedState(prev => ({ ...prev, status: 'ready', progress: 100 }));
              break;
          }
        },
        { timeout: UPDATE_DOWNLOAD_TIMEOUT_MS },
      );
      setSharedState(prev => ({ ...prev, status: 'ready', progress: 100 }));
    } catch (e) {
      setSharedState(prev => ({
        ...prev,
        status: 'error',
        progress: undefined,
        errorStage: 'download',
        ...extractError(e),
      }));
      return;
    }
  }, []);

  const restart = useCallback(async () => {
    try {
      await relaunch();
    } catch (e) {
      setSharedState(prev => ({
        ...prev,
        status: 'error',
        progress: undefined,
        errorStage: 'install',
        ...extractError(e),
      }));
    }
  }, []);

  useEffect(() => {
    listeners.add(setState);
    setState(sharedState);
    return () => {
      listeners.delete(setState);
    };
  }, []);

  useEffect(() => {
    if (!checkOnMount || autoCheckStarted) return;
    autoCheckStarted = true;
    let fired = false;
    const timer = setTimeout(() => {
      fired = true;
      void checkForUpdate();
    }, 5000);
    return () => {
      clearTimeout(timer);
      if (!fired) {
        autoCheckStarted = false;
      }
    };
  }, [checkOnMount, checkForUpdate]);

  return { ...state, checkForUpdate, downloadAndInstall, restart };
}
