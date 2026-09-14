import { invoke, isTauri } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

export interface DesktopMonitor { id: string; width: number; height: number; primary: boolean }
export interface ScreenCapture { stream: MediaStream; stop(): void }

export function useDesktopMonitors(enabled = true) {
  const [monitors, setMonitors] = useState<DesktopMonitor[]>([]);
  useEffect(() => {
    let cancelled = false;
    if (enabled && isTauri()) {
      void invoke<DesktopMonitor[]>('list_desktop_monitors_cmd')
        .then(items => { if (!cancelled) setMonitors(Array.isArray(items) ? items : []); })
        .catch(() => { if (!cancelled) setMonitors([]); });
    }
    return () => { cancelled = true; };
  }, [enabled]);
  return monitors;
}

/** Use the system window picker, or an explicitly selected native monitor. */
export async function openScreenCapture(monitorId?: string): Promise<ScreenCapture> {
  if (!monitorId) {
    const stream = await navigator.mediaDevices.getDisplayMedia({ video: { frameRate: 2, displaySurface: 'monitor' }, audio: false });
    return { stream, stop: () => stream.getTracks().forEach(track => track.stop()) };
  }
  const canvas = document.createElement('canvas');
  const context = canvas.getContext('2d');
  if (!context || !canvas.captureStream) throw new Error('Desktop capture is unavailable in this WebView');
  let stopped = false;
  let pending = false;
  let timer: ReturnType<typeof setInterval> | undefined;
  let stream: MediaStream | undefined;
  const stop = () => { stopped = true; clearInterval(timer); stream?.getTracks().forEach(track => track.stop()); };
  const draw = async () => {
    const base64 = await invoke<string>('capture_desktop_monitor_cmd', { monitorId });
    if (stopped) return;
    const frame = new Image();
    frame.src = `data:image/jpeg;base64,${base64}`;
    await frame.decode();
    if (stopped) return;
    if (canvas.width !== frame.naturalWidth || canvas.height !== frame.naturalHeight) {
      canvas.width = frame.naturalWidth; canvas.height = frame.naturalHeight;
    }
    context.drawImage(frame, 0, 0);
  };
  await draw();
  stream = canvas.captureStream(2);
  timer = setInterval(() => {
    if (stopped || stream?.getVideoTracks().every(track => track.readyState === 'ended')) { stop(); return; }
    if (pending) return;
    pending = true;
    void draw().catch(() => {
      const tracks = stream?.getVideoTracks() ?? [];
      stop();
      tracks.forEach(track => track.dispatchEvent(new Event('ended')));
    }).finally(() => { pending = false; });
  }, 1000);
  return { stream, stop };
}
