import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Square } from 'lucide-react';
import { useTranslation } from '../i18n';
import './DesktopControlStatusPage.css';

interface DesktopActivity { conversationId: string; runId: string; callId: string; toolName: string }

export function DesktopControlStatusPage() {
  const { t } = useTranslation();
  const [activities, setActivities] = useState<DesktopActivity[]>([]);
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    let cancelled = false;
    let revision = 0;
    const unlisten = listen<DesktopActivity[]>('desktop-control:status', event => {
      revision++;
      if (!cancelled) setActivities(Array.isArray(event.payload) ? event.payload : []);
    });
    const before = revision;
    void invoke<DesktopActivity[]>('desktop_control_status_cmd').then(snapshot => {
      if (!cancelled && revision === before) setActivities(Array.isArray(snapshot) ? snapshot : []);
    }).catch(() => {});
    return () => { cancelled = true; void unlisten.then(dispose => dispose()).catch(() => {}); };
  }, []);
  const active = activities[0];
  if (!active) return null;
  const stop = async () => {
    setStopping(true); setError('');
    try { await invoke('stop_desktop_control_cmd', { conversationId: active.conversationId }); }
    catch (error) { setError(String(error)); }
    finally { setStopping(false); }
  };
  return <div className="nexa-control-status flex h-screen items-center gap-3 px-4 text-text-primary" data-testid="computer-use-status" data-observing={active.toolName !== 'computer_control'}>
    <svg className="nexa-control-bond" viewBox="0 0 42 42" fill="none" aria-hidden="true">
      <circle className="bond-orbit" cx="21" cy="21" r="19" stroke="currentColor" strokeOpacity=".14" />
      <path d="M24 14l3-3a7 7 0 0110 10l-7 7a7 7 0 01-10 0M18 28l-3 3A7 7 0 015 21l7-7a7 7 0 0110 0" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" opacity=".75" />
      <path className="bond-pulse" d="M24 14l3-3a7 7 0 0110 10l-7 7a7 7 0 01-10 0M18 28l-3 3A7 7 0 015 21l7-7a7 7 0 0110 0" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
      <path d="M16 26l10-10" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" />
      <circle cx="16" cy="26" r="2.5" fill="currentColor" />
      <circle cx="26" cy="16" r="2.5" fill="currentColor" />
    </svg>
    <div className="min-w-0 flex-1" role="status">
      <p className="text-xs font-semibold">Nexa</p>
      <p className="truncate text-sm">{t(active.toolName === 'computer_control' ? 'chat.computerUseOperating' : 'chat.computerUseObserving')}</p>
      {error && <p className="truncate text-xs text-red-500" role="alert">{error}</p>}
    </div>
    <button className="flex shrink-0 items-center gap-1 rounded-md border border-border px-2 py-1 text-xs hover:bg-surface-2 disabled:opacity-50" onClick={() => void stop()} disabled={stopping}><Square size={12} />{t('chat.stop')}</button>
    <span className="nexa-control-thread" aria-hidden="true" />
  </div>;
}
