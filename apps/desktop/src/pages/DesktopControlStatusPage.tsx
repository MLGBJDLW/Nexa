import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Monitor, Square } from 'lucide-react';
import { useTranslation } from '../i18n';

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
  return <div className="flex h-screen items-center gap-3 border border-border bg-surface-1 px-4 text-text-primary" data-testid="computer-use-status">
    <Monitor className="shrink-0 text-accent" size={20} />
    <div className="min-w-0 flex-1" role="status">
      <p className="text-xs font-semibold">Nexa</p>
      <p className="truncate text-sm">{t(active.toolName === 'computer_control' ? 'chat.computerUseOperating' : 'chat.computerUseObserving')}</p>
      {error && <p className="truncate text-xs text-red-500" role="alert">{error}</p>}
    </div>
    <button className="flex shrink-0 items-center gap-1 rounded-md border border-border px-2 py-1 text-xs hover:bg-surface-2 disabled:opacity-50" onClick={() => void stop()} disabled={stopping}><Square size={12} />{t('chat.stop')}</button>
  </div>;
}
