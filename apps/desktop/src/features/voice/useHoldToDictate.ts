import { useCallback, useEffect, useRef, type MouseEvent, type PointerEvent } from 'react';

type Hold = {
  id: number; x: number; y: number; target: HTMLTextAreaElement; active: boolean;
  timer?: ReturnType<typeof setTimeout>;
};
export type DictationControls = { start: () => void; finish: () => void; cancel: () => void };

/** A stationary primary press owns capture only after the hold threshold. */
export function useHoldToDictate(options: {
  enabled: boolean; onStart: () => void; onFinish: () => void; onCancel: () => void;
  pointerTypes?: readonly string[];
}) {
  const current = useRef(options);
  current.current = options;
  const held = useRef<Hold | null>(null);
  const suppressClick = useRef(false);
  const lastPress = useRef({ time: -Infinity, x: 0, y: 0 });
  const end = useCallback((cancel: boolean) => {
    const press = held.current;
    if (!press) return;
    held.current = null;
    clearTimeout(press.timer);
    if (!press.active) return;
    if (press.target.hasPointerCapture(press.id)) press.target.releasePointerCapture(press.id);
    if (cancel) current.current.onCancel(); else current.current.onFinish();
  }, []);
  useEffect(() => {
    const release = (event: globalThis.PointerEvent) => { if (event.pointerId === held.current?.id) end(false); };
    const cancelPointer = (event: globalThis.PointerEvent) => { if (event.pointerId === held.current?.id) end(true); };
    const cancel = () => end(true);
    const hidden = () => { if (document.hidden) cancel(); };
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') cancel(); };
    window.addEventListener('pointerup', release, true);
    window.addEventListener('pointercancel', cancelPointer, true);
    window.addEventListener('blur', cancel);
    window.addEventListener('pagehide', cancel);
    window.addEventListener('keydown', escape);
    document.addEventListener('visibilitychange', hidden);
    return () => {
      window.removeEventListener('pointerup', release, true);
      window.removeEventListener('pointercancel', cancelPointer, true);
      window.removeEventListener('blur', cancel);
      window.removeEventListener('pagehide', cancel);
      window.removeEventListener('keydown', escape);
      document.removeEventListener('visibilitychange', hidden);
      cancel();
    };
  }, [end]);
  return {
    onPointerDown(event: PointerEvent<HTMLTextAreaElement>) {
      suppressClick.current = false;
      const target = event.currentTarget;
      if (!event.isPrimary || event.button !== 0 || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) { end(true); return; }
      if (!current.current.enabled || held.current || target.disabled || target.readOnly || target.selectionStart !== target.selectionEnd) return;
      if (current.current.pointerTypes && !current.current.pointerTypes.includes(event.pointerType)) return;
      const now = performance.now();
      const previous = lastPress.current;
      lastPress.current = { time: now, x: event.clientX, y: event.clientY };
      if (now - previous.time < 350 && Math.hypot(event.clientX - previous.x, event.clientY - previous.y) < 10) return;
      const press: Hold = { id: event.pointerId, x: event.clientX, y: event.clientY, target, active: false };
      held.current = press;
      press.timer = setTimeout(() => {
        if (held.current !== press) return;
        if (!current.current.enabled || !target.isConnected || document.hidden || target.selectionStart !== target.selectionEnd) { end(true); return; }
        try { target.setPointerCapture(press.id); } catch { end(true); return; }
        press.active = true; suppressClick.current = true;
        current.current.onStart();
      }, 480);
    },
    onPointerMove(event: PointerEvent<HTMLTextAreaElement>) {
      const press = held.current;
      if (!press || press.id !== event.pointerId) return;
      if (event.buttons !== 1 || ((!press.active || event.pointerType === 'mouse') && Math.hypot(event.clientX - press.x, event.clientY - press.y) > 10)) end(true);
    },
    onPointerLeave(event: PointerEvent<HTMLTextAreaElement>) { if (held.current?.id === event.pointerId && !held.current.active) end(true); },
    onBlur() { end(true); },
    onLostPointerCapture(event: PointerEvent<HTMLTextAreaElement>) { if (held.current?.id === event.pointerId) end(true); },
    onClickCapture(event: MouseEvent<HTMLTextAreaElement>) {
      if (suppressClick.current) { suppressClick.current = false; event.preventDefault(); event.stopPropagation(); }
    },
    onContextMenu(event: MouseEvent<HTMLTextAreaElement>) {
      if (held.current?.active || (held.current && !event.currentTarget.value)) event.preventDefault();
      else end(true);
    },
  };
}
