import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

/** Project the existing CSS theme, including plugins, into native feedback. */
export function connectNativeComputerUseAppearance(): () => void {
  if (!('__TAURI_INTERNALS__' in window)) return () => {};
  try { if (getCurrentWindow().label !== 'main') return () => {}; }
  catch { return () => {}; }
  const media = window.matchMedia('(prefers-reduced-motion: reduce)');
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 1;
  const context = canvas.getContext('2d', { willReadFrequently: true });
  if (!context) return () => {};
  let disposed = false;
  let scheduled = false;
  let previous = '';
  let delivery = Promise.resolve();
  const sync = () => {
    scheduled = false;
    if (disposed) return;
    const style = getComputedStyle(document.documentElement);
    const color = style.getPropertyValue('--color-accent').trim();
    if (!color || !CSS.supports('color', color)) return;
    context.clearRect(0, 0, 1, 1);
    context.fillStyle = color;
    context.fillRect(0, 0, 1, 1);
    const [red, green, blue] = context.getImageData(0, 0, 1, 1).data;
    const appearance = {
      accent: [red, green, blue],
      reducedMotion: media.matches || style.getPropertyValue('--theme-duration-scale').trim() === '0',
    };
    const key = JSON.stringify(appearance);
    if (key === previous) return;
    previous = key;
    delivery = delivery.then(async () => {
      if (!disposed) await invoke('set_desktop_control_appearance_cmd', appearance);
    }).catch((error: unknown) => { console.warn('Unable to update native computer-use appearance', error); });
  };
  const schedule = () => {
    if (scheduled || disposed) return;
    scheduled = true;
    // Unlike animation frames this also updates while the main window is hidden.
    queueMicrotask(sync);
  };
  const observer = new MutationObserver(schedule);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'style'] });
  media.addEventListener('change', schedule);
  schedule();
  return () => { disposed = true; observer.disconnect(); media.removeEventListener('change', schedule); };
}
