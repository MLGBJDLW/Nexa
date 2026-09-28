import { invoke } from '@tauri-apps/api/core';
import { Update as TauriUpdate, type Update as TauriUpdateInstance } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { useState, useEffect, useCallback } from 'react';
import { resolveReleaseNotes } from './releaseNotes';

export const UPDATE_SOURCES = ['github', 'ghfast', 'custom'] as const;
export type UpdateSource = typeof UPDATE_SOURCES[number];

interface UpdateState {
  status: 'idle' | 'checking' | 'available' | 'downloading' | 'ready' | 'error' | 'up-to-date';
  source: UpdateSource;
  customMirror: string;
  version?: string;
  notes?: string;
  progress?: number;
  error?: string;
  errorCode?: string | number | null;
  errorDetail?: { stack?: string };
  errorStage?: 'check' | 'download' | 'install';
  lastCheckedAt?: string;
}

interface TauriUpdateMetadata {
  rid: number;
  currentVersion: string;
  version: string;
  date?: string;
  body?: string;
  rawJson: Record<string, unknown>;
}

const UPDATE_SOURCE_STORAGE_KEY = 'nexa-update-source';
const UPDATE_MIRROR_STORAGE_KEY = 'nexa-update-custom-mirror';
const DEFAULT_UPDATE_SOURCE: UpdateSource = 'github';
const UPDATE_CHECK_TIMEOUT_MS = 90_000;
const UPDATE_DOWNLOAD_TIMEOUT_MS = 600_000;

function isUpdateSource(value: string | null): value is UpdateSource {
  return UPDATE_SOURCES.includes(value as UpdateSource);
}

function readStoredUpdateSource(): UpdateSource {
  if (typeof window === 'undefined') return DEFAULT_UPDATE_SOURCE;
  try {
    const value = window.localStorage.getItem(UPDATE_SOURCE_STORAGE_KEY);
    return isUpdateSource(value) ? value : DEFAULT_UPDATE_SOURCE;
  } catch {
    return DEFAULT_UPDATE_SOURCE;
  }
}

function persistUpdateSource(source: UpdateSource) {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(UPDATE_SOURCE_STORAGE_KEY, source);
  } catch {
    // Ignore storage failures; the current session still uses the selected source.
  }
}

async function checkUpdateFromSource(source: UpdateSource, customMirror: string): Promise<TauriUpdateInstance | null> {
  const metadata = await invoke<TauriUpdateMetadata | null>('check_update_from_source_cmd', {
    source,
    customMirror: source === 'custom' ? customMirror : null,
    timeout: UPDATE_CHECK_TIMEOUT_MS,
  });
  return metadata ? new TauriUpdate(metadata) : null;
}

let sharedSource = readStoredUpdateSource();
let sharedCustomMirror = (() => {
  try { return window.localStorage.getItem(UPDATE_MIRROR_STORAGE_KEY) ?? ''; } catch { return ''; }
})();
let sharedState: UpdateState = { status: 'idle', source: sharedSource, customMirror: sharedCustomMirror };
let sharedUpdate: TauriUpdateInstance | null = null;
let sourceRevision = 0;
let checkRevision = 0;
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

  const setUpdateSource = useCallback((source: UpdateSource) => {
    if (source === sharedSource || sharedState.status === 'downloading') return;
    sourceRevision += 1;
    sharedSource = source;
    persistUpdateSource(source);
    void sharedUpdate?.close().catch(() => {});
    sharedUpdate = null;
    setSharedState({ status: 'idle', source, customMirror: sharedCustomMirror });
  }, []);

  const setCustomMirror = useCallback((value: string) => {
    if (value === sharedCustomMirror || sharedState.status === 'downloading') return;
    sharedCustomMirror = value;
    sourceRevision += 1;
    try { window.localStorage.setItem(UPDATE_MIRROR_STORAGE_KEY, value); } catch { /* session only */ }
    void sharedUpdate?.close().catch(() => {});
    sharedUpdate = null;
    setSharedState({ status: 'idle', source: sharedSource, customMirror: value });
  }, []);

  const checkForUpdate = useCallback(async (sourceOverride?: UpdateSource) => {
    const source = sourceOverride ?? sharedSource;
    if (sharedState.status === 'downloading') return null;
    const revision = sourceRevision;
    const checkId = ++checkRevision;
    const customMirror = sharedCustomMirror;
    const stale = () => revision !== sourceRevision || checkId !== checkRevision;
    setSharedState({ status: 'checking', source, customMirror });
    try {
      const update = await checkUpdateFromSource(source, customMirror);
      const lastCheckedAt = new Date().toISOString();
      if (stale()) {
        void update?.close().catch(() => {});
        return null;
      }
      if (update) {
        const notes = await resolveReleaseNotes(update);
        if (stale()) {
          void update.close().catch(() => {});
          return null;
        }
        void sharedUpdate?.close().catch(() => {});
        sharedUpdate = update;
        setSharedState({
          status: 'available',
          source,
          customMirror,
          version: update.version,
          notes,
          lastCheckedAt,
        });
        return update;
      } else {
        void sharedUpdate?.close().catch(() => {});
        sharedUpdate = null;
        setSharedState({ status: 'up-to-date', source, customMirror, lastCheckedAt });
        return null;
      }
    } catch (e) {
      if (stale()) {
        return null;
      }
      void sharedUpdate?.close().catch(() => {});
      sharedUpdate = null;
      setSharedState({ status: 'error', source, customMirror, errorStage: 'check', lastCheckedAt: new Date().toISOString(), ...extractError(e) });
      return null;
    }
  }, []);

  const downloadAndInstall = useCallback(async () => {
    if (sharedState.status === 'downloading' || sharedState.status === 'checking') return;
    let update = sharedUpdate;
    const source = sharedSource;
    if (!update) {
      try {
        update = await checkForUpdate(source);
        if (update) sharedUpdate = update;
      } catch (e) {
        setSharedState({ status: 'error', source, customMirror: sharedCustomMirror, errorStage: 'check', lastCheckedAt: new Date().toISOString(), ...extractError(e) });
        return;
      }
      if (!update) return;
    }

    setSharedState(prev => ({
      ...prev,
      status: 'downloading',
      source,
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
              // Download completion precedes signature verification/installation.
              setSharedState(prev => ({ ...prev, progress: 100 }));
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
  }, [checkForUpdate]);

  const restart = useCallback(async () => {
    try {
      await relaunch();
    } catch (e) {
      setSharedState(prev => ({
        ...prev,
        status: 'error',
        progress: undefined,
        source: sharedSource,
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

  return { ...state, setUpdateSource, setCustomMirror, checkForUpdate, downloadAndInstall, restart };
}
