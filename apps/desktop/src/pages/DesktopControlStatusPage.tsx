import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Square } from 'lucide-react';
import { Logo } from '../components/Logo';
import { useTranslation } from '../i18n';
import './DesktopControlStatusPage.css';

interface DesktopActivity { conversationId: string; runId: string; callId: string; toolName: string; phase?: 'controlling' | 'waiting' }

function controlActivities(value: DesktopActivity[]): DesktopActivity[] {
  return Array.isArray(value) ? value.filter(activity => activity.toolName === 'computer_control') : [];
}

export function DesktopControlStatusPage() {
  const { t } = useTranslation();
  const [activities, setActivities] = useState<DesktopActivity[]>([]);
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    document.documentElement.classList.add('nexa-control-surface');
    return () => document.documentElement.classList.remove('nexa-control-surface');
  }, []);
  useEffect(() => {
    let cancelled = false;
    let revision = 0;
    const unlisten = listen<DesktopActivity[]>('desktop-control:status', event => {
      revision++;
      if (!cancelled) setActivities(controlActivities(event.payload));
    });
    void unlisten.then(async () => {
      if (cancelled) return;
      const before = revision;
      const snapshot = await invoke<DesktopActivity[]>('desktop_control_status_cmd');
      if (!cancelled && revision === before) setActivities(controlActivities(snapshot));
    }).catch(() => {});
    return () => { cancelled = true; void unlisten.then(dispose => dispose()).catch(() => {}); };
  }, []);
  const active = activities[0];
  if (!active) return null;
  const stop = async () => {
    setStopping(true); setError('');
    try { await invoke('stop_desktop_control_cmd', { conversationId: active.conversationId, runId: active.runId }); }
    catch (error) { setError(String(error)); }
    finally { setStopping(false); }
  };
  return <div key={active.runId} className="nexa-control-status flex h-screen items-center gap-3 px-4 text-text-primary" data-testid="computer-use-status" data-waiting={active.phase === 'waiting'}>
    <div className="nexa-control-bond" aria-hidden="true"><span className="nexa-control-aura" /><Logo size={38} decorative /></div>
    <div className="min-w-0 flex-1" role="status">
      <p className="text-xs font-semibold">Nexa</p>
      <p className="truncate text-sm">{t(active.phase === 'waiting' ? 'chat.computerUseWaiting' : 'chat.computerUseOperating')}</p>
      {error && <p className="truncate text-xs text-red-500" role="alert">{error}</p>}
    </div>
    <button aria-label={t('chat.computerUseStop')} title={t('chat.computerUseStop')} className="flex shrink-0 items-center gap-1 rounded-md border border-border px-2 py-1 text-xs hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-50" onClick={() => void stop()} disabled={stopping}><Square size={12} />{t('chat.stop')}</button>
    <span className="nexa-control-thread" aria-hidden="true" />
  </div>;
}
