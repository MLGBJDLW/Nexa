import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useDesktopMonitors, useDesktopWindows } from '../../lib/screenCapture';
import { shareFromSystemPicker } from './systemPickerShare';
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
  const [changing, setChanging] = useState(false);
  const change = useRef<AbortController | null>(null);
  const [error, setError] = useState('');
  useEffect(() => () => {
    opening.current++;
    if (viewer.current) void invoke('remote_phone_screen_stop_cmd', {sessionId:viewer.current});
  }, [deviceId]);
  const windows = useDesktopWindows(expanded && connected);
  const monitors = useDesktopMonitors(expanded && connected);
  const systemPicker = isTauri() && typeof navigator.mediaDevices?.getDisplayMedia === 'function';
  const primary = monitors.find(monitor => monitor.primary) ?? monitors[0];
  const selected = source || (primary ? `monitor:${primary.id}` : systemPicker ? 'picker' : '');
  const windowSelected = windows.windows.some(window => window.id === selected);
  const selectedAvailable = windowSelected || (selected === 'picker' && systemPicker)
    || monitors.some(monitor => `monitor:${monitor.id}` === selected);
  const supported = monitors.length > 0 || windows.supported === true || systemPicker;
  useEffect(() => {
    setChanging(false);
    return () => {
      change.current?.abort();
      change.current = null;
    };
  }, [deviceId, connected, expanded]);
  useEffect(() => {
    let disposed = false;
    if (!connected) { opening.current++; setSharing(false); setView(null); return; }
    const refresh = () => void invoke<ScreenShareStatus[]>('remote_screen_status_cmd').then(shares => {
      const share = Array.isArray(shares) ? shares.find(share => share.deviceId === deviceId) : undefined;
      if (!disposed) { setSharing(Boolean(share)); if (share) { setSource(share.source.startsWith('picker:') ? 'picker' : share.source); setControl(share.control); } }
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
    if (change.current) return;
    const pending = new AbortController();
    change.current = pending;
    setChanging(true);
    try {
      setError('');
      if (start && selected === 'picker') await shareFromSystemPicker(deviceId, pending.signal);
      else await invoke('remote_screen_share_cmd', {deviceId, source: start ? selected : null, control: windowSelected && control});
      if (!pending.signal.aborted) setSharing(start);
    } catch (error) { if (!pending.signal.aborted) setError(String(error)); }
    finally {
      if (change.current === pending) { change.current = null; setChanging(false); }
    }
  };
  return <div className="w-full space-y-3">
    <button className={remoteButton} disabled={!connected} onClick={() => { if (expanded) { opening.current++; if (view) void transport.stop().catch(error => setError(String(error))); setView(null); } setExpanded(value => !value); }}>{t('remote.screenTitle')}</button>
    {expanded && <div className="space-y-3">
      {error && <p role="alert" className="text-xs text-danger">{error}</p>}
      {!supported && windows.supported === false ? <p className="text-xs text-text-secondary">{t('remote.screenHostUnsupported')}</p> : <><label className="block space-y-2 text-xs"><span>{t('remote.screenShareSource')}</span><select className={remoteField} value={selected} onChange={event => { setSource(event.target.value); setControl(false); }} disabled={sharing || changing}>
        {!selectedAvailable && <option value={selected} disabled>{t('remote.screenChooseSource')}</option>}
        {monitors.map((monitor, index) => <option key={monitor.id} value={`monitor:${monitor.id}`}>{t('live.screen')} {index + 1} · {monitor.width} × {monitor.height}</option>)}
        {windows.windows.map(window => <option key={window.id} value={window.id}>{window.title} · {window.appName}</option>)}
        {systemPicker && <option value="picker">{t('chat.shareWindow')}</option>}
      </select></label>
      {windowSelected && <label className="flex items-center gap-2 text-xs"><input type="checkbox" checked={control} onChange={event => setControl(event.target.checked)} disabled={sharing || changing}/>{t('remote.screenControl')}</label>}</>}
      <div className="flex flex-wrap gap-2">{(sharing || supported) && <button className={remoteButton} disabled={changing || (!sharing && !selectedAvailable)} onClick={() => void share(!sharing)}>{t(sharing ? 'remote.screenStop' : 'remote.screenShare')}</button>}
      <button className={remoteButton} onClick={() => void togglePhone()}>{t('remote.screenViewPhone')}</button></div>
      {sharing && connected && <p role="status" className="text-xs text-accent">{t('remote.screenSharing')}</p>}
      {view && connected && <DeviceScreen transport={transport} onClose={() => setView(null)}/>}
    </div>}
  </div>;
}
