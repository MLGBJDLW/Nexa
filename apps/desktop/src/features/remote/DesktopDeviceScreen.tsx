import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useDesktopWindows } from '../../lib/screenCapture';
import { useTranslation } from '../../i18n';
import { DeviceScreen, type ScreenFrame } from './DeviceScreen';
import { remoteButton, remoteField } from './remoteUi';
import type { ScreenShareStatus } from './RemoteScreenStatus';
export function DesktopDeviceScreen({ deviceId, connected }: {deviceId: string; connected: boolean}) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const [view, setView] = useState<string | null>(null);
  const opening = useRef(0);
  const viewer = useRef(view); viewer.current = view;
  const [source, setSource] = useState('');
  const [control, setControl] = useState(false);
  const [sharing, setSharing] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => () => {
    opening.current++;
    if (viewer.current) void invoke('remote_phone_screen_stop_cmd', {sessionId:viewer.current});
  }, [deviceId]);
  const windows = useDesktopWindows(expanded && connected);
  useEffect(() => {
    let disposed = false;
    if (!connected) { opening.current++; setSharing(false); setView(null); return; }
    const refresh = () => void invoke<ScreenShareStatus[]>('remote_screen_status_cmd').then(shares => {
      const share = Array.isArray(shares) ? shares.find(share => share.deviceId === deviceId) : undefined;
      if (!disposed) { setSharing(Boolean(share)); if (share) { setSource(share.source); setControl(share.control); } }
    }).catch(error => { if (!disposed) setError(String(error)); });
    refresh(); let unlisten: (() => void) | undefined;
    void listen('remote:screen-changed', refresh).then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, [connected, expanded, deviceId]);
  const transport = useMemo(() => ({phone:true, frame: () => invoke<ScreenFrame>('remote_phone_screen_cmd', {sessionId:view, arguments:{action:'screenshot'}}), input: (arguments_: Record<string, unknown>) => invoke('remote_phone_screen_cmd', {sessionId:view, arguments: arguments_}), stop: () => invoke('remote_phone_screen_stop_cmd', {sessionId:view})}), [view]);
  const togglePhone = async () => {
    const mine = ++opening.current;
    if (view) { await transport.stop(); setView(null); return; }
    try {
      setError('');
      const id = await invoke<string>('remote_phone_screen_open_cmd', {deviceId});
      if (mine !== opening.current) { await invoke('remote_phone_screen_stop_cmd', {sessionId:id}); return; }
      setView(id);
    } catch (error) { if (mine === opening.current) setError(String(error)); }
  };
  const share = async (start: boolean) => {
    try { setError(''); await invoke('remote_screen_share_cmd', {deviceId, source: start ? source : null, control}); setSharing(start); }
    catch (error) { setError(String(error)); }
  };
  return <div className="w-full space-y-3">
    <button className={remoteButton} disabled={!connected} onClick={() => { if (expanded) { opening.current++; if (view) void transport.stop().catch(error => setError(String(error))); setView(null); } setExpanded(value => !value); }}>{t('remote.screenTitle')}</button>
    {expanded && <div className="space-y-3">
      {error && <p role="alert" className="text-xs text-danger">{error}</p>}
      {windows.supported === false ? <p className="text-xs text-text-secondary">{t('remote.screenHostUnsupported')}</p> : <><label className="block space-y-2 text-xs"><span>{t('remote.screenShareWindow')}</span><select className={remoteField} value={source} onChange={event => setSource(event.target.value)} disabled={sharing}>
        <option value="">{t('remote.screenChooseWindow')}</option>{windows.windows.map(window => <option key={window.id} value={window.id}>{window.title} · {window.appName}</option>)}
      </select></label>
      <label className="flex items-center gap-2 text-xs"><input type="checkbox" checked={control} onChange={event => setControl(event.target.checked)} disabled={sharing}/>{t('remote.screenControl')}</label></>}
      <div className="flex flex-wrap gap-2">{(sharing || windows.supported === true) && <button className={remoteButton} disabled={!sharing && !source} onClick={() => void share(!sharing)}>{t(sharing ? 'remote.screenStop' : 'remote.screenShare')}</button>}
      <button className={remoteButton} onClick={() => void togglePhone()}>{t('remote.screenViewPhone')}</button></div>
      {sharing && connected && <p role="status" className="text-xs text-accent">{t('remote.screenSharing')}</p>}
      {view && connected && <DeviceScreen transport={transport} onClose={() => setView(null)}/>}
    </div>}
  </div>;
}
