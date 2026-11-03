import { useLayoutEffect, useRef, useState, type PointerEvent, type ReactNode } from 'react';
import { Menu, SquarePen, X } from 'lucide-react';
import { useTranslation } from '../../i18n';

export function MobileChatHeader({ center, onMenu, onNew, disabled }: { center: ReactNode; onMenu: () => void; onNew?: () => void; disabled?: boolean }) {
  const { t } = useTranslation();
  return <header className="native-chat-header"><button className="mobile-icon-button" onClick={onMenu} aria-label={t('remote.conversation')}><Menu size={21}/></button><div className="native-chat-heading">{center}</div>{onNew ? <button className="mobile-icon-button" onClick={onNew} disabled={disabled} aria-label={t('remote.newChat')}><SquarePen size={21}/></button> : <span className="mobile-icon-spacer"/>}</header>;
}

/** Native dialog owns focus and background inertness; presence keeps exit motion intact. */
export function MobileDialog({ title, side = false, open = true, children, onClose }: { title: string; side?: boolean; open?: boolean; children: ReactNode; onClose: () => void }) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDialogElement>(null);
  const close = useRef(onClose); close.current = onClose;
  const [present, setPresent] = useState(open);
  const motion = useRef<Animation | null>(null);
  const drag = useRef<{ id: number; x: number; y: number; at: number; distance: number; active: boolean } | null>(null);
  const suppressClick = useRef(false);
  const reduced = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  useLayoutEffect(() => { if (open) setPresent(true); }, [open]);
  useLayoutEffect(() => {
    const node = dialog.current;
    if (!node || !present) return;
    const outside = side ? 'translateX(-100%)' : 'translateY(100%)';
    const current = node.open ? getComputedStyle(node).transform : outside;
    motion.current?.cancel();
    if (!node.open) node.showModal();
    node.dataset.state = open ? 'open' : 'closing';
    const animation = node.animate([{ transform: current }, { transform: open ? 'translate(0,0)' : outside }], {
      duration: reduced() ? 0 : open ? 280 : 200, easing: open ? 'cubic-bezier(.22,1,.36,1)' : 'cubic-bezier(.4,0,1,1)', fill: 'both',
    });
    motion.current = animation;
    void animation.finished.then(() => {
      node.style.removeProperty('transform'); node.style.removeProperty('--mobile-backdrop-opacity');
      animation.cancel();
      if (!open) { node.close(); setPresent(false); }
    }).catch(() => {});
    return () => animation.cancel();
  }, [open, present, side]);
  useLayoutEffect(() => () => { dialog.current?.close(); }, []);
  const start = (event: PointerEvent<HTMLDialogElement>) => {
    suppressClick.current = false;
    const target = event.target as HTMLElement;
    if (!open || event.button !== 0 || (!side && !target.closest('[data-mobile-drag-handle]')) || target.closest('input,textarea,select,button[aria-label]')) return;
    drag.current = { id:event.pointerId,x:event.clientX,y:event.clientY,at:performance.now(),distance:0,active:false };
  };
  const move = (event: PointerEvent<HTMLDialogElement>) => {
    const state = drag.current, node = dialog.current;
    if (!state || state.id !== event.pointerId || !node) return;
    const distance = side ? state.x - event.clientX : event.clientY - state.y;
    const cross = side ? Math.abs(event.clientY-state.y) : Math.abs(event.clientX-state.x);
    if (!state.active && cross > 10 && cross > Math.abs(distance)) { drag.current = null; return; }
    if (!state.active && distance > 8 && distance > cross) {
      state.active = true; motion.current?.cancel(); node.setPointerCapture(event.pointerId);
    }
    if (!state.active) return;
    state.distance = Math.max(0,distance); suppressClick.current = true;
    node.style.transform = side ? `translateX(-${state.distance}px)` : `translateY(${state.distance}px)`;
    const size = side ? node.clientWidth : node.clientHeight;
    node.style.setProperty('--mobile-backdrop-opacity',String(.28*Math.max(0,1-state.distance/size)));
  };
  const finish = (event: PointerEvent<HTMLDialogElement>, cancelled = false) => {
    if (cancelled) suppressClick.current = false;
    const state = drag.current, node = dialog.current; drag.current = null;
    if (!state?.active || !node) return;
    if (node.hasPointerCapture(event.pointerId)) node.releasePointerCapture(event.pointerId);
    const size = side ? node.clientWidth : node.clientHeight;
    if (!cancelled && (state.distance > Math.min(110,size*.25) || (state.distance > 24 && state.distance/Math.max(1,performance.now()-state.at) > .55))) { close.current(); return; }
    motion.current = node.animate([{transform:node.style.transform},{transform:'translate(0,0)'}],{duration:reduced()?0:180,easing:'cubic-bezier(.22,1,.36,1)'});
    node.style.removeProperty('transform'); node.style.removeProperty('--mobile-backdrop-opacity');
  };
  if (!present) return null;
  return <dialog ref={dialog} className={side ? 'mobile-dialog mobile-drawer' : 'mobile-dialog mobile-sheet'} aria-label={title} onCancel={event => { event.preventDefault(); close.current(); }}
    onPointerDown={start} onPointerMove={move} onPointerUp={event=>finish(event)} onPointerCancel={event=>finish(event,true)}
    onClickCapture={event=>{if(suppressClick.current){event.preventDefault();event.stopPropagation();suppressClick.current=false;}}}
    onClick={event => { if (event.target === event.currentTarget) close.current(); }}>
    <div className="mobile-dialog-body">{!side && <div className="mobile-sheet-handle" data-mobile-drag-handle aria-hidden="true"><span/></div>}<header data-mobile-drag-handle><h2>{title}</h2><button className="mobile-icon-button" onClick={()=>close.current()} aria-label={t('common.close')}><X size={20}/></button></header>{children}</div>
  </dialog>;
}

export interface MobileChatPresentation {
  heading: ReactNode;
  drawerFooter: ReactNode;
  caption: string;
  onConfigure: () => void;
  onFiles: () => void;
}
