import { useDraggable } from '@dnd-kit/core';
import { Archive, GripVertical } from 'lucide-react';
import { motion, useIsPresent, usePresenceData, useReducedMotion } from 'framer-motion';
import { useTranslation } from '../../i18n';
import { useEffect, useRef, type ReactNode, type PointerEvent } from 'react';

/** Horizontal intent belongs to the row; vertical motion remains list scrolling. */
export function MobileConversationRow({ children, id, title, preview, onArchive, onMove }: {
  children: ReactNode; id: string; title: string; preview: boolean; onArchive?: () => void; onMove?: () => void;
}) {
  const { t } = useTranslation();
  const present = useIsPresent();
  const movedId = usePresenceData();
  const { setNodeRef, listeners, isDragging } = useDraggable({ id, disabled: !onMove || !present });
  const reducedMotion = useReducedMotion();
  const drag = useRef<{ id: number; x: number; y: number; distance: number; horizontal: boolean } | null>(null);
  const suppressClick = useRef(false);
  const archiveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => { if (archiveTimer.current) clearTimeout(archiveTimer.current); }, []);
  const finish = (event: PointerEvent<HTMLDivElement>, cancelled = false) => {
    const state = drag.current; drag.current = null;
    if (!state) return;
    if (event.currentTarget.hasPointerCapture(state.id)) event.currentTarget.releasePointerCapture(state.id);
    event.currentTarget.dataset.swiping = 'false';
    event.currentTarget.style.setProperty('--swipe-distance', '0px');
    event.currentTarget.style.setProperty('--swipe-progress', '0');
    // Let the gesture's synthesized click be consumed by this row before the
    // next dialog enters the top layer (otherwise it can dismiss that dialog).
    if (!cancelled && state.horizontal && state.distance >= 80) archiveTimer.current = setTimeout(() => onArchive?.(), 0);
  };
  return <motion.div ref={setNodeRef} className="mobile-swipe-row" data-mobile-swipe-row data-conversation-id={id}
    data-archive-preview={preview || (!present && movedId !== id) || undefined}
    data-project-dragging={isDragging || undefined} inert={!present}
    onMouseDown={event => { if (!(event.target as Element).closest('[data-archive-button]')) listeners?.onMouseDown?.(event); }}
    onTouchStart={event => { if (!(event.target as Element).closest('[data-archive-button]')) listeners?.onTouchStart?.(event); }}
    onContextMenu={event => { if (onMove) event.preventDefault(); }}
    exit={{ height: 0, opacity: 0, transition: { duration: reducedMotion ? 0 : .2, delay: reducedMotion ? 0 : .16 } }}
    onPointerDown={event => {
      suppressClick.current = false;
      if (!onArchive || event.button !== 0 || (event.target as HTMLElement).closest('.mobile-icon-button')) return;
      drag.current = { id: event.pointerId, x: event.clientX, y: event.clientY, distance: 0, horizontal: false };
    }}
    onPointerMove={event => {
      const state = drag.current;
      if (!state || state.id !== event.pointerId || isDragging) return;
      const distance = state.x - event.clientX, cross = Math.abs(state.y - event.clientY);
      if (!state.horizontal && cross > 10 && cross > Math.abs(distance)) { drag.current = null; return; }
      if (!state.horizontal && distance > 12 && distance > cross) {
        state.horizontal = true; suppressClick.current = true;
        event.currentTarget.setPointerCapture(event.pointerId);
      }
      if (state.horizontal) {
        state.distance = Math.max(0, distance);
        event.currentTarget.dataset.swiping = 'true';
        event.currentTarget.style.setProperty('--swipe-distance', `${Math.min(140, state.distance)}px`);
        event.currentTarget.style.setProperty('--swipe-progress', String(Math.min(1, state.distance / 100)));
      }
    }}
    onPointerUp={event => finish(event, isDragging)} onPointerCancel={event => finish(event, true)}
    onClickCapture={event => { if (suppressClick.current) { event.preventDefault(); event.stopPropagation(); suppressClick.current = false; } }}>
    {onArchive && <div className="mobile-swipe-action" aria-hidden="true"><Archive size={20}/><span>{t('chat.archive')}</span></div>}
    <div className="mobile-conversation-row mobile-swipe-content">{children}
      {onMove && <button className="mobile-icon-button mobile-project-grip" aria-label={`${t('project.moveToProject')}: ${title}`} onClick={onMove}><GripVertical size={17}/></button>}
      {onArchive && <button data-archive-button className="mobile-icon-button" aria-label={`${t('chat.archive')}: ${title}`} onClick={onArchive}><Archive size={16}/></button>}
    </div>
  </motion.div>;
}
