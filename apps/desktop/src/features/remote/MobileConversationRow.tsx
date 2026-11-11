import { useDraggable } from '@dnd-kit/core';
import { Archive, ArchiveRestore, GripVertical, Trash2 } from 'lucide-react';
import { motion, useIsPresent, usePresenceData, useReducedMotion } from 'framer-motion';
import { useTranslation } from '../../i18n';
import { useEffect, useRef, type ReactNode, type PointerEvent } from 'react';

/** One action surface follows the finger, then fills the row before confirmation. */
export function MobileConversationRow({ children, id, title, preview, restoring, onArchive, onDelete, onRestore, onMove, disabled }: {
  children: ReactNode; id: string; title: string; preview?: boolean; restoring?: boolean; disabled?: boolean;
  onArchive?: () => void; onDelete?: () => void; onRestore?: () => void; onMove?: () => void;
}) {
  const { t } = useTranslation();
  const present = useIsPresent();
  const movedId = usePresenceData();
  const { setNodeRef, listeners, isDragging } = useDraggable({ id, disabled: !onMove || !present });
  const reducedMotion = useReducedMotion();
  const drag = useRef<{ id: number; x: number; y: number; distance: number; horizontal: boolean } | null>(null);
  const suppressClick = useRef(false);
  const actionTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => { if (actionTimer.current) clearTimeout(actionTimer.current); }, []);
  const trailing = onDelete ?? onArchive;
  const reset = (row: HTMLElement) => {
    delete row.dataset.swiping;
    delete row.dataset.swipeCommitting;
    row.style.removeProperty('--swipe-distance');
    row.style.removeProperty('--swipe-progress');
  };
  const commit = (row: HTMLElement, restore: boolean) => {
    const action = restore ? onRestore : trailing;
    if (!action || disabled || actionTimer.current) return;
    row.dataset.swiping = restore ? 'right' : 'left';
    row.dataset.swipeCommitting = 'true';
    row.style.setProperty('--swipe-distance', `${row.clientWidth}px`);
    row.style.setProperty('--swipe-progress', '1');
    actionTimer.current = setTimeout(() => {
      actionTimer.current = null;
      action();
      reset(row);
    }, reducedMotion ? 0 : 180);
  };
  const finish = (event: PointerEvent<HTMLDivElement>, cancelled = false) => {
    const state = drag.current;
    if (!state || event.pointerId !== state.id) return;
    drag.current = null;
    if (event.currentTarget.hasPointerCapture(state.id)) event.currentTarget.releasePointerCapture(state.id);
    if (!cancelled && !disabled && state.horizontal && Math.abs(state.distance) >= 80) {
      commit(event.currentTarget, state.distance < 0);
    } else reset(event.currentTarget);
  };
  const actionButton = (restore: boolean) => {
    const Icon = restore ? ArchiveRestore : onDelete ? Trash2 : Archive;
    const label = t(restore ? 'chat.unarchive' : onDelete ? 'common.delete' : 'chat.archive');
    return <button type="button" data-archive-button={!restore && !onDelete || undefined}
      className={`mobile-icon-button mobile-swipe-action ${restore ? 'restore' : onDelete ? 'delete' : 'archive'}`}
      disabled={disabled} aria-label={`${label}: ${title}`} onClick={event => commit(event.currentTarget.closest<HTMLElement>('.mobile-swipe-row')!, restore)}>
      <Icon size={16}/><span aria-hidden="true">{label}</span>
    </button>;
  };
  return <motion.div ref={setNodeRef} className="mobile-swipe-row" data-mobile-swipe-row data-conversation-id={id}
    data-archive-preview={preview || (!present && movedId !== id) || undefined}
    data-restore-preview={restoring || (!present && !!onRestore && movedId === id) || undefined}
    data-project-dragging={isDragging || undefined} inert={!present}
    onMouseDown={event => { if (!(event.target as Element).closest('.mobile-swipe-action')) listeners?.onMouseDown?.(event); }}
    onTouchStart={event => { if (!(event.target as Element).closest('.mobile-swipe-action')) listeners?.onTouchStart?.(event); }}
    onContextMenu={event => { if (onMove) event.preventDefault(); }}
    exit={{ height: 0, opacity: 0, transition: { duration: reducedMotion ? 0 : .2, delay: reducedMotion ? 0 : .16 } }}
    onPointerDown={event => {
      if (!event.isPrimary || event.button !== 0) return;
      suppressClick.current = false;
      if (disabled || actionTimer.current || (!trailing && !onRestore) || (event.target as Element).closest('.mobile-project-grip')) return;
      drag.current = { id: event.pointerId, x: event.clientX, y: event.clientY, distance: 0, horizontal: false };
    }}
    onPointerMove={event => {
      const state = drag.current;
      if (!state || state.id !== event.pointerId || isDragging) return;
      const distance = state.x - event.clientX, cross = Math.abs(state.y - event.clientY);
      if (!state.horizontal && cross > 10 && cross > Math.abs(distance)) { drag.current = null; return; }
      if (!state.horizontal && Math.abs(distance) > 12 && Math.abs(distance) > cross && (distance > 0 ? trailing : onRestore)) {
        state.horizontal = true; suppressClick.current = true;
        event.currentTarget.setPointerCapture(event.pointerId);
      }
      if (state.horizontal) {
        state.distance = (distance > 0 ? trailing : onRestore) ? distance : 0;
        event.currentTarget.dataset.swiping = state.distance < 0 ? 'right' : 'left';
        event.currentTarget.style.setProperty('--swipe-distance', `${Math.min(event.currentTarget.clientWidth, Math.abs(state.distance))}px`);
        event.currentTarget.style.setProperty('--swipe-progress', String(Math.min(1, Math.abs(state.distance) / Math.max(80, event.currentTarget.clientWidth - 44))));
      }
    }}
    onPointerUp={event => finish(event, isDragging)} onPointerCancel={event => finish(event, true)}
    onLostPointerCapture={event => {
      // Touch starts with implicit capture on the child button. Transferring
      // it to this row emits a bubbling loss from that button, not a cancel.
      if (event.target === event.currentTarget) finish(event, true);
    }}
    onClickCapture={event => { if (suppressClick.current && event.detail) { event.preventDefault(); event.stopPropagation(); suppressClick.current = false; } }}>
    <div className="mobile-conversation-row">{onRestore && actionButton(true)}{children}
      {onMove && <button className="mobile-icon-button mobile-project-grip" aria-label={`${t('project.moveToProject')}: ${title}`} onClick={onMove}><GripVertical size={17}/></button>}
      {trailing && actionButton(false)}
    </div>
  </motion.div>;
}
