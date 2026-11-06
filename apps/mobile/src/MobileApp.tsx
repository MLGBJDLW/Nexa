import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ArrowRight, Check, ChevronDown, FolderOpen, Loader2, Monitor, Plus, ScanLine, Settings, Smartphone } from 'lucide-react';
import { Logo } from '../../desktop/src/components/Logo';
import { useTranslation } from '../../desktop/src/i18n';
import { MobileChatHeader, MobileDialog, type MobileChatPresentation } from '../../desktop/src/features/remote/MobileChatShell';
import type { RemoteConnection } from '../../desktop/src/features/remote/remoteClient';
import { hostLabel, NativeClient, type NativeHost } from './nativeClient';
import type { PhoneSettings } from './localClient';
import { phoneChatProjects } from './phoneChatProjects';

const LocalChat = lazy(() => import('./LocalChat').then(module => ({ default: module.LocalChat })));
const ComputerChat = lazy(() => import('./ComputerChat'));
const PhoneSettingsSheet = lazy(() => import('./PhoneSettingsSheet'));
export interface PhoneConnection { id: string; name: string; provider: string; model: string; baseUrl: string | null }
export interface ConnectionSyncSummary { supported: boolean; checkedAt: string; connections: number; received: number; sent: number; conflicts: number; deviceOnly: number }
export interface PhoneCapabilities { accessibility: boolean; accessibilityEnabled?: boolean; accessibilityStatus?: 'connected' | 'disconnected' | 'disabled'; notifications: boolean; restrictedSettingsMayApply?: boolean; batteryOptimized?: boolean; backgroundRestricted?: boolean; manufacturer?: string; androidVersion?: number; screenCapture?: boolean; assistantOverlay?: boolean; remoteControlHost?: string }
export interface MobileState { version: string; settings: PhoneSettings | null; activeConnectionId?: string | null; connections?: PhoneConnection[]; connectionSync?: Record<string, ConnectionSyncSummary>; hosts: NativeHost[]; capabilities: PhoneCapabilities }

export function MobileApp() {
  const { t } = useTranslation();
  const [state, setState] = useState<MobileState | null>(null);
  const [target, setTarget] = useState(() => localStorage.getItem('nexa.mobile.target') || 'phone');
  const [setupSkipped, setSetupSkipped] = useState(() => localStorage.getItem('nexa.mobile.setupSkipped') === 'true');
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsLoaded, setSettingsLoaded] = useState(false);
  const [targetsOpen, setTargetsOpen] = useState(false);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [notificationTarget, setNotificationTarget] = useState<MobileChatPresentation['notificationTarget']>(null);
  const onNotificationHandled = useCallback(() => setNotificationTarget(null), []);
  useEffect(() => {
    let active = true;
    const consume = async () => {
      if (document.visibilityState === 'hidden') return;
      const destination = await invoke<NonNullable<MobileChatPresentation['notificationTarget']>>('mobile_notification_target').catch(() => null);
      if (!active || !destination?.conversationId) return;
      setNotificationTarget(destination); setTarget('phone');
      setSettingsOpen(false); setTargetsOpen(false); setDrawerOpen(false); setFilesOpen(false);
      setSetupSkipped(true);
    };
    void consume();
    window.addEventListener('nexa:notificationOpen', consume);
    document.addEventListener('visibilitychange', consume);
    return () => {
      active = false;
      window.removeEventListener('nexa:notificationOpen', consume);
      document.removeEventListener('visibilitychange', consume);
    };
  }, []);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [pairUrl, setPairUrl] = useState('');
  const pairRequest = useRef(crypto.randomUUID());
  const [connection, setConnection] = useState<RemoteConnection>({ phase: 'connecting', endpoint: null });
  const [files, setFiles] = useState<string[]>([]);
  const [filesOpen, setFilesOpen] = useState(false);
  // One host-wide foreground check. The Rust scheduler claims persisted
  // occurrences and admits at most one phone executor, regardless of chat.
  useEffect(() => {
    let checking = false;
    const tick = async () => {
      if (checking || document.visibilityState !== 'visible') return;
      checking = true;
      try { await invoke('mobile_local', { action: 'scheduleTick', input: {} }); }
      catch (error) { console.warn('Phone schedule check failed', error); }
      finally { checking = false; }
    };
    void tick();
    const timer = setInterval(tick, 30_000);
    document.addEventListener('visibilitychange', tick);
    return () => { clearInterval(timer); document.removeEventListener('visibilitychange', tick); };
  }, []);
  const reads = useRef(0);
  const clients = useRef(new Map<string, { client: NativeClient; stop: () => void }>());
  const [, setClientVersion] = useState(0);
  const syncRequests = useRef(new Map<string, { again: boolean; promise: Promise<ConnectionSyncSummary> }>());
  const [syncBusy, setSyncBusy] = useState<Record<string, boolean>>({});
  const [syncErrors, setSyncErrors] = useState<Record<string, string>>({});
  const [syncResults, setSyncResults] = useState<Record<string, ConnectionSyncSummary>>({});
  const host = state?.hosts.find(item => item.id === target);
  const client = clients.current.get(target)?.client ?? null;
  const refresh = useCallback(async () => {
    const generation = ++reads.current;
    try { const next = await invoke<MobileState>('mobile_state'); if (generation === reads.current) setState(next); }
    catch (error) { if (generation === reads.current) setError(String(error)); }
  }, []);
  const syncConnections = useCallback((id: string, followup = false): Promise<ConnectionSyncSummary> => {
    const pending = syncRequests.current.get(id);
    if (pending) { pending.again ||= followup; return pending.promise; }
    const entry = { again: false, promise: undefined as unknown as Promise<ConnectionSyncSummary> };
    setSyncBusy(current => ({ ...current, [id]: true }));
    setSyncErrors(current => ({ ...current, [id]: '' }));
    entry.promise = (async () => {
      for (;;) {
        entry.again = false;
        try {
          const result = await invoke<ConnectionSyncSummary>('mobile_sync_connections', { hostId: id });
          setSyncResults(current => ({ ...current, [id]: result }));
          await refresh();
          if (!entry.again || !clients.current.has(id)) return result;
        } catch (error) {
          if (!entry.again || !clients.current.has(id)) throw error;
        }
      }
    })().catch(error => { setSyncErrors(current => ({ ...current, [id]: String(error) })); throw error; })
      .finally(() => { syncRequests.current.delete(id); setSyncBusy(current => ({ ...current, [id]: false })); });
    syncRequests.current.set(id, entry);
    return entry.promise;
  }, [refresh]);
  const pairingIds = state?.hosts.map(item => `${item.id}:${item.device.id}`).sort().join('|') ?? '';
  useEffect(() => {
    const hosts = state?.hosts ?? [];
    for (const [id, entry] of clients.current) if (!hosts.some(host => host.id === id && host.device.id === entry.client.paired.device.id)) { entry.stop(); clients.current.delete(id); }
    for (const host of hosts) if (!clients.current.has(host.id)) {
      const client = new NativeClient(host);
      const changed = () => { void syncConnections(host.id, true).catch(() => {}); };
      const off = client.subscribe(event => { if (event.event === 'connections:changed' || event.event === 'connection:resync') changed(); });
      clients.current.set(host.id, { client, stop: () => { off(); client.close(); } });
      void client.start().catch(error => setSyncErrors(current => ({ ...current, [host.id]: String(error) })));
    }
    setClientVersion(value => value + 1);
  }, [pairingIds, syncConnections]);
  useEffect(() => () => { clients.current.forEach(entry => entry.stop()); clients.current.clear(); }, []);
  useEffect(() => {
    let active = true;
    let stop: (() => void) | undefined;
    void listen<{ changed: boolean }>('mobile:connections', ({ payload }) => {
      if (!active) return;
      void refresh();
      if (payload.changed) for (const id of clients.current.keys()) void syncConnections(id, true).catch(() => {});
    }).then(off => { if (active) stop = off; else off(); });
    return () => { active = false; stop?.(); };
  }, [refresh, syncConnections]);
  useEffect(() => {
    void refresh();
    const foreground = () => { if (document.visibilityState !== 'hidden') { void refresh(); for (const id of clients.current.keys()) void syncConnections(id).catch(() => {}); } };
    document.addEventListener('visibilitychange', foreground);
    window.addEventListener('focus', foreground);
    return () => { document.removeEventListener('visibilitychange', foreground); window.removeEventListener('focus', foreground); };
  }, [refresh, syncConnections]);
  useEffect(() => { localStorage.setItem('nexa.mobile.target', target); }, [target]);
  useEffect(() => {
    if (!client) return;
    return client.subscribeConnection(setConnection);
  }, [client]);
  const action = async (run: () => Promise<unknown>) => {
    setBusy(true); setError('');
    try { await run(); await refresh(); } catch (error) { setError(String(error)); } finally { setBusy(false); }
  };
  const openSettings = useCallback(() => { setSettingsLoaded(true); setSettingsOpen(true); }, []);
  const openFiles = useCallback(() => { void invoke<string[]>('mobile_files', { action: 'list' }).then(files => { setFiles(files); setFilesOpen(true); }).catch(error => setError(String(error))); }, []);
  const targetLabel = target === 'phone' ? t('remote.mobilePhone') : t('remote.mobileComputer');
  const heading = <button className="mobile-target-button" onClick={() => setTargetsOpen(true)} aria-label={t('remote.mobileTarget')}><Logo size={26} decorative/><span><strong>Nexa</strong><small>{targetLabel}<ChevronDown size={12}/></small></span></button>;
  const drawerFooter = <><button onClick={() => setTargetsOpen(true)}><Smartphone size={18}/>{t('remote.mobileTarget')}</button><button onClick={openFiles}><FolderOpen size={18}/>{t('remote.mobileFiles')}</button><button onClick={openSettings}><Settings size={18}/>{t('remote.mobileSettings')}</button></>;
  const projectStore = useMemo(() => phoneChatProjects(target === 'phone' ? null : target), [target]);
  const presentation: MobileChatPresentation = { heading, drawerFooter, projectStore, notificationTarget:target==='phone'?notificationTarget:null, onNotificationHandled, caption: target === 'phone' ? t('remote.mobileLocalBoundary') : t('remote.mobileRemoteBoundary'), onConfigure: openSettings, onFiles: openFiles };
  const activeChat = !!state && (target === 'phone' ? !!state.settings || setupSkipped : !!client);
  const skipSetup = () => { localStorage.setItem('nexa.mobile.setupSkipped', 'true'); setSetupSkipped(true); setTarget('phone'); };
  const scanPairing = () => action(async () => {
    const result = await invoke<{ url: string | null }>('mobile_scan_pairing');
    if (!result.url) return;
    const url = new URL(result.url);
    const fragment = new URLSearchParams(url.hash.slice(1));
    if (url.protocol !== 'https:' || url.username || url.password || !fragment.get('pair') || !fragment.get('server')) throw new Error(t('remote.mobileScanInvalid'));
    setPairUrl(result.url); pairRequest.current = crypto.randomUUID();
  });
  const loading = <div className="mobile-loading"><Loader2 size={22} className="animate-spin"/></div>;

  return <main className="mobile-app" data-testid="native-mobile-app">
    {error && <div className="mobile-error" role="alert">{error}<button onClick={() => setError('')} aria-label={t('common.close')}>×</button></div>}
    {!activeChat && <MobileChatHeader center={heading} onMenu={() => setDrawerOpen(true)}/>}
    {!state ? loading : target === 'phone' ? state.settings || setupSkipped ? <Suspense fallback={loading}><LocalChat settings={state.settings} presentation={presentation}/></Suspense>
      : <section className="mobile-onboarding"><Logo size={56}/><h1>{t('remote.mobilePhoneTitle')}</h1><p>{t('remote.mobilePhoneHint')}</p><div className="mobile-setup-choices"><button className="mobile-primary" onClick={() => setTarget('pair')}><Monitor size={17}/>{t('remote.mobileConnectComputer')}<ArrowRight size={17}/></button>{state.hosts.map(computer => <button key={computer.id} onClick={() => setTarget(computer.id)}><Monitor size={17}/>{hostLabel(computer)}<ArrowRight size={17}/></button>)}<button onClick={openSettings}><Smartphone size={17}/>{t('remote.mobileConfigure')}<ArrowRight size={17}/></button><button onClick={skipSetup}>{t('remote.mobileSkipSetup')}</button></div></section>
      : client ? <Suspense fallback={loading}><ComputerChat key={client.paired.id} client={client} presentation={presentation}/></Suspense> : host ? loading
      : <section className="mobile-pair"><Logo size={40}/><h1>{t('remote.mobilePairTitle')}</h1><p>{t('remote.mobilePairHint')}</p><button className="mobile-scan-button mobile-primary" disabled={busy} onClick={() => void scanPairing()}><ScanLine size={20}/>{t('remote.mobileScanPairing')}</button><form onSubmit={event => { event.preventDefault(); void action(async () => {
        const paired = await invoke<NativeHost>('mobile_pair', { url: pairUrl.trim(), name: 'Nexa Mobile', requestId: pairRequest.current });
        setPairUrl(''); pairRequest.current = crypto.randomUUID(); setTarget(paired.id);
      }); }}><label>{t('remote.mobilePairLink')}<textarea aria-label={t('remote.mobilePairLink')} rows={3} value={pairUrl} onChange={event => { setPairUrl(event.target.value); pairRequest.current = crypto.randomUUID(); }} placeholder="https://…/phone#pair=…" autoCapitalize="none" autoCorrect="off" spellCheck={false}/></label><p className="mobile-section-hint">{t('remote.mobileSyncTrustedHint')}</p><button className="mobile-primary" disabled={busy || !pairUrl.trim()}>{busy ? <Loader2 size={17} className="animate-spin"/> : <ArrowRight size={17}/>} {t('remote.pair')}</button></form><p className="mobile-footnote">{t('remote.pairAuthority')}</p><button className="mobile-skip" onClick={skipSetup}>{t('remote.mobileSkipSetup')}</button></section>}
    {!activeChat && <p className="mobile-version">Nexa Mobile {state?.version ?? '0.1.0'}</p>}
    {activeChat && target !== 'phone' && connection.phase === 'revoked' && <p className="mobile-error">{t('remote.revokedHint')}</p>}
    <MobileDialog open={drawerOpen} side title="Nexa" onClose={() => setDrawerOpen(false)}><div className="mobile-drawer-footer" onClick={() => setDrawerOpen(false)}>{drawerFooter}</div></MobileDialog>
    <MobileDialog open={targetsOpen} title={t('remote.mobileTarget')} onClose={() => setTargetsOpen(false)}><div className="mobile-target-list">
      <button className={target === 'phone' ? 'selected' : ''} onClick={() => { setTarget('phone'); setTargetsOpen(false); }}><Smartphone size={21}/><span><strong>{t('remote.mobilePhone')}</strong><small>{t('remote.mobileLocalBoundary')}</small></span>{target === 'phone' && <Check size={18}/>}</button>
      {state?.hosts.map(item => <button key={item.id} className={target === item.id ? 'selected' : ''} onClick={() => { setTarget(item.id); setTargetsOpen(false); }}><Monitor size={21}/><span><strong>{t('remote.mobileComputer')}</strong><small>{hostLabel(item)}</small></span>{target === item.id && <Check size={18}/>}</button>)}
      <button onClick={() => { setTarget('pair'); setTargetsOpen(false); }}><Plus size={21}/><span>{t('remote.addDevice')}</span><ArrowRight size={17}/></button>
    </div></MobileDialog>
    {settingsLoaded && state && <Suspense fallback={null}><PhoneSettingsSheet open={settingsOpen} state={{ ...state, connectionSync: { ...state.connectionSync, ...syncResults } }} host={host} syncBusy={syncBusy} syncErrors={syncErrors} onSync={id => syncConnections(id)} onClose={() => setSettingsOpen(false)} onChanged={refresh} onForgot={() => { setTarget('phone'); void refresh(); }}/></Suspense>}
    <MobileDialog open={filesOpen} title={t('remote.mobileFiles')} onClose={() => setFilesOpen(false)}><p className="mobile-section-hint">{t('remote.mobileFilesHint')}</p><div className="mobile-file-list">{files.map(path => <button key={path} onClick={() => void action(() => invoke('mobile_files', { action: 'share', relativePath: path }))}><FolderOpen size={18}/><span>{path}</span><ArrowRight size={16}/></button>)}</div></MobileDialog>
  </main>;
}
