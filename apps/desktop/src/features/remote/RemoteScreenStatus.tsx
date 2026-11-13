import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Link } from 'react-router';
import { useTranslation } from '../../i18n';
import { remoteButton } from './remoteUi';
export interface ScreenShareStatus {deviceId: string; active: boolean; control: boolean; source: string}
export function RemoteScreenStatus() {
  const { t } = useTranslation();
  const [shares, setShares] = useState<ScreenShareStatus[]>([]);
  useEffect(() => {
    let disposed = false; let pending = false; let again = false;
    const refresh = async () => {
      if (pending) { again = true; return; } pending = true;
      try { const shares = await invoke<ScreenShareStatus[]>('remote_screen_status_cmd'); if (!disposed) setShares(Array.isArray(shares) ? shares : []); }
      catch { /* Native IPC is unavailable in browser previews. */ }
      finally { pending = false; if (again && !disposed) { again = false; void refresh(); } }
    };
    void refresh();
    let unlisten: (() => void) | undefined;
    void listen('remote:screen-changed', () => void refresh()).then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, []);
  if (!shares.length) return null;
  return <div role="status" className="fixed right-4 top-4 z-50 flex max-w-[calc(100vw-6rem)] flex-wrap items-center gap-2 rounded-xl border border-accent/40 bg-surface-1 p-3 text-xs shadow-xl">
    <span className="h-2 w-2 rounded-full bg-accent motion-safe:animate-pulse"/><Link to="/remote">{t('remote.screenSharing')}</Link>
    <button className={remoteButton} onClick={() => void Promise.allSettled(shares.map(share => invoke('remote_screen_share_cmd',{deviceId:share.deviceId,source:null,control:false}))).then(() => invoke<ScreenShareStatus[]>('remote_screen_status_cmd')).then(value => setShares(Array.isArray(value) ? value : [])).catch(() => {})}>{t('remote.screenStop')}</button>
  </div>;
}
