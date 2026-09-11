import { useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { RemoteManifest } from './remoteClient';
export interface RemoteDesktopStatus {
  publicRoutes?: { provider:'localhostRun' | 'pinggy' | 'cloudflare'; phase:'connecting' | 'checking' | 'standby' | 'ready' | 'retrying' | 'stopped'; urls:string[]; latencyMs:number | null; error:string | null }[];
  enabled: boolean;
  preparing: boolean;
  connectedDeviceIds: string[];
  pairingDeviceId: string | null;
  manifest: RemoteManifest | null;
  devices: { id: string; name: string; createdAt: string }[];
  warning: string | null;
  certificatePath: string | null;
  tunnelRunning: boolean;
}
let snapshot: { status: RemoteDesktopStatus | null; error: string | null } = {
  status: null,
  error: null,
};
const listeners = new Set<() => void>();
let cleanup: (() => void) | null = null;
let pending: Promise<void> | null = null;
let refreshAgain = false;
function publish(next: typeof snapshot) {
  if (JSON.stringify(next) === JSON.stringify(snapshot)) return;
  snapshot = next;
  for (const listener of listeners) listener();
}
export function refreshRemoteStatus(): Promise<void> {
  if (pending) { refreshAgain = true; return pending; }
  pending = (async () => {
    do {
      refreshAgain = false;
      let timer: ReturnType<typeof setTimeout> | undefined;
      try {
        const status = await Promise.race([
          invoke<RemoteDesktopStatus>('remote_status_cmd'),
          new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error('Remote status did not respond.')), 10_000); }),
        ]);
        publish({ status, error: null });
      } catch (error) { publish({ status: snapshot.status, error: String(error) }); }
      finally { if (timer) clearTimeout(timer); }
    } while (refreshAgain);
  })().finally(() => { pending = null; });
  return pending;
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  if (!cleanup) {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen('remote:status-changed', () => {
      if (!disposed) void refreshRemoteStatus();
    })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => {});
    void refreshRemoteStatus();
    const timer = setInterval(() => {
      if (snapshot.status?.enabled || snapshot.status?.preparing) void refreshRemoteStatus();
    }, 15_000);
    cleanup = () => {
      disposed = true;
      unlisten?.();
      clearInterval(timer);
    };
  }
  return () => {
    listeners.delete(listener);
    if (!listeners.size) {
      cleanup?.();
      cleanup = null;
    }
  };
}
export function useRemoteDesktopStatus() {
  return useSyncExternalStore(subscribe, () => snapshot);
}
