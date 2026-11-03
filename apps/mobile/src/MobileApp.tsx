import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowRight, Check, ChevronDown, FolderOpen, Loader2, Monitor, Plus, Settings, Smartphone } from 'lucide-react';
import { Logo } from '../../desktop/src/components/Logo';
import { useTranslation } from '../../desktop/src/i18n';
import { MobileChatHeader, MobileDialog, type MobileChatPresentation } from '../../desktop/src/features/remote/MobileChatShell';
import type { RemoteConnection } from '../../desktop/src/features/remote/remoteClient';
import { NativeClient, type NativeHost } from './nativeClient';
import type { PhoneSettings } from './localClient';

const LocalChat = lazy(() => import('./LocalChat').then(module => ({ default: module.LocalChat })));
const ComputerChat = lazy(() => import('./ComputerChat'));
const PhoneSettingsSheet = lazy(() => import('./PhoneSettingsSheet'));
export interface MobileState { version: string; settings: PhoneSettings | null; hosts: NativeHost[]; capabilities: { accessibility: boolean; notifications: boolean } }

export function MobileApp() {
  const { t } = useTranslation();
  const [state, setState] = useState<MobileState | null>(null);
  const [target, setTarget] = useState(() => localStorage.getItem('nexa.mobile.target') || 'phone');
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [targetsOpen, setTargetsOpen] = useState(false);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [pairUrl, setPairUrl] = useState('');
  const pairRequest = useRef(crypto.randomUUID());
  const [connection, setConnection] = useState<RemoteConnection>({ phase: 'connecting', endpoint: null });
  const [files, setFiles] = useState<string[] | null>(null);
  const reads = useRef(0);
  const host = state?.hosts.find(item => item.id === target);
  const client = useMemo(() => host ? new NativeClient(host) : null, [host?.id]);
  const refresh = useCallback(async () => {
    const generation = ++reads.current;
    try { const next = await invoke<MobileState>('mobile_state'); if (generation === reads.current) setState(next); }
    catch (error) { if (generation === reads.current) setError(String(error)); }
  }, []);
  useEffect(() => {
    void refresh();
    const foreground = () => { if (document.visibilityState !== 'hidden') void refresh(); };
    document.addEventListener('visibilitychange', foreground);
    return () => document.removeEventListener('visibilitychange', foreground);
  }, [refresh]);
  useEffect(() => { localStorage.setItem('nexa.mobile.target', target); }, [target]);
  useEffect(() => {
    if (!client) return;
    const off = client.subscribeConnection(setConnection);
    void client.start().catch(error => setError(String(error)));
    return () => { off(); client.close(); };
  }, [client]);
  const action = async (run: () => Promise<unknown>) => {
    setBusy(true); setError('');
    try { await run(); await refresh(); } catch (error) { setError(String(error)); } finally { setBusy(false); }
  };
  const openSettings = useCallback(() => setSettingsOpen(true), []);
  const openFiles = useCallback(() => { void invoke<string[]>('mobile_files', { action: 'list' }).then(setFiles).catch(error => setError(String(error))); }, []);
  const targetLabel = target === 'phone' ? t('remote.mobilePhone') : t('remote.mobileComputer');
  const heading = <button className="mobile-target-button" onClick={() => setTargetsOpen(true)} aria-label={t('remote.mobileTarget')}><Logo size={26} decorative/><span><strong>Nexa</strong><small>{targetLabel}<ChevronDown size={12}/></small></span></button>;
  const drawerFooter = <><button onClick={() => setTargetsOpen(true)}><Smartphone size={18}/>{t('remote.mobileTarget')}</button><button onClick={openFiles}><FolderOpen size={18}/>{t('remote.mobileFiles')}</button><button onClick={openSettings}><Settings size={18}/>{t('remote.mobileSettings')}</button></>;
  const presentation: MobileChatPresentation = { heading, drawerFooter, caption: target === 'phone' ? t('remote.mobileLocalBoundary') : t('remote.mobileRemoteBoundary'), onConfigure: openSettings, onFiles: openFiles };
  const activeChat = !!state && (target === 'phone' ? !!state.settings : !!client);
  const loading = <div className="mobile-loading"><Loader2 size={22} className="animate-spin"/></div>;

  return <main className="mobile-app" data-testid="native-mobile-app">
    {error && <div className="mobile-error" role="alert">{error}<button onClick={() => setError('')} aria-label={t('common.close')}>×</button></div>}
    {!activeChat && <MobileChatHeader center={heading} onMenu={() => setDrawerOpen(true)}/>}
    {!state ? loading : target === 'phone' ? state.settings ? <Suspense fallback={loading}><LocalChat settings={state.settings} presentation={presentation}/></Suspense>
      : <section className="mobile-onboarding"><Logo size={56}/><h1>{t('remote.mobilePhoneTitle')}</h1><p>{t('remote.mobilePhoneHint')}</p><button className="mobile-primary" onClick={openSettings}>{t('remote.mobileConfigure')}<ArrowRight size={17}/></button></section>
      : client ? <Suspense fallback={loading}><ComputerChat client={client} presentation={presentation}/></Suspense>
      : <section className="mobile-pair"><Logo size={40}/><h1>{t('remote.mobilePairTitle')}</h1><p>{t('remote.mobilePairHint')}</p><form onSubmit={event => { event.preventDefault(); void action(async () => {
        const paired = await invoke<NativeHost>('mobile_pair', { url: pairUrl.trim(), name: 'Nexa Mobile', requestId: pairRequest.current });
        setPairUrl(''); pairRequest.current = crypto.randomUUID(); setTarget(paired.id);
      }); }}><label>{t('remote.mobilePairLink')}<textarea aria-label={t('remote.mobilePairLink')} rows={3} value={pairUrl} onChange={event => { setPairUrl(event.target.value); pairRequest.current = crypto.randomUUID(); }} placeholder="https://…/phone#pair=…" autoCapitalize="none" autoCorrect="off" spellCheck={false}/></label><button className="mobile-primary" disabled={busy || !pairUrl.trim()}>{busy ? <Loader2 size={17} className="animate-spin"/> : <ArrowRight size={17}/>} {t('remote.pair')}</button></form><p className="mobile-footnote">{t('remote.pairAuthority')}</p></section>}
    {!activeChat && <p className="mobile-version">Nexa Mobile {state?.version ?? '0.1.0'}</p>}
    {activeChat && target !== 'phone' && connection.phase === 'revoked' && <p className="mobile-error">{t('remote.revokedHint')}</p>}
    {drawerOpen && <MobileDialog side title="Nexa" onClose={() => setDrawerOpen(false)}><div className="mobile-drawer-footer" onClick={() => setDrawerOpen(false)}>{drawerFooter}</div></MobileDialog>}
    {targetsOpen && <MobileDialog title={t('remote.mobileTarget')} onClose={() => setTargetsOpen(false)}><div className="mobile-target-list">
      <button className={target === 'phone' ? 'selected' : ''} onClick={() => { setTarget('phone'); setTargetsOpen(false); }}><Smartphone size={21}/><span><strong>{t('remote.mobilePhone')}</strong><small>{t('remote.mobileLocalBoundary')}</small></span>{target === 'phone' && <Check size={18}/>}</button>
      {state?.hosts.map(item => <button key={item.id} className={target === item.id ? 'selected' : ''} onClick={() => { setTarget(item.id); setTargetsOpen(false); }}><Monitor size={21}/><span><strong>{t('remote.mobileComputer')}</strong><small>{new URL(item.endpoint).hostname}</small></span>{target === item.id && <Check size={18}/>}</button>)}
      <button onClick={() => { setTarget('pair'); setTargetsOpen(false); }}><Plus size={21}/><span>{t('remote.addDevice')}</span><ArrowRight size={17}/></button>
    </div></MobileDialog>}
    {settingsOpen && state && <Suspense fallback={loading}><PhoneSettingsSheet state={state} host={host} onClose={() => setSettingsOpen(false)} onChanged={refresh} onForgot={() => { setTarget('phone'); void refresh(); }}/></Suspense>}
    {files && <MobileDialog title={t('remote.mobileFiles')} onClose={() => setFiles(null)}><p className="mobile-section-hint">{t('remote.mobileFilesHint')}</p><div className="mobile-file-list">{files.map(path => <button key={path} onClick={() => void action(() => invoke('mobile_files', { action: 'share', relativePath: path }))}><FolderOpen size={18}/><span>{path}</span><ArrowRight size={16}/></button>)}</div></MobileDialog>}
  </main>;
}
