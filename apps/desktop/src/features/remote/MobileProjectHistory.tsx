import { useEffect, useRef, useState } from 'react';
import {
  DndContext, DragOverlay, MouseSensor, TouchSensor, useDroppable, useSensor, useSensors,
  type CollisionDetection,
} from '@dnd-kit/core';
import { AnimatePresence } from 'framer-motion';
import { Check, Folder, FolderOpen, MessageSquare, MessagesSquare, MoreHorizontal, Pencil, Plus, Trash2 } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { MobileDialog } from './MobileChatShell';
import { MobileConversationRow } from './MobileConversationRow';

interface Project { id: string; name: string }
interface Chat { id: string; title: string; model: string }
const dropButtonAt = (x: number, y: number) =>
  document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-project-drop]');

export interface MobileProjectView {
  projects: Project[];
  memberships: Readonly<Record<string, string>>;
  selectedId: string | null;
  select(id: string | null): void;
  create(name: string): Promise<void>;
  rename(id: string, name: string): Promise<void>;
  remove(id: string): Promise<void>;
  move(conversationId: string, projectId: string | null): Promise<void>;
  loading?: boolean;
  error?: string;
  retry?(): void;
}

// The title remains a scroll / swipe surface. The grip can drag immediately
// in either direction; on the title touch users first hold to pick up a chat.
class ChatMouseSensor extends MouseSensor {
  constructor(props: ConstructorParameters<typeof MouseSensor>[0]) {
    const grip = (props.event.target as Element).closest('.mobile-project-grip');
    super({ ...props, options: { activationConstraint: grip ? { distance: 5 } : { distance: { y: 7 }, tolerance: { x: 16 } } } });
  }
}
class ChatTouchSensor extends TouchSensor {
  constructor(props: ConstructorParameters<typeof TouchSensor>[0]) {
    const grip = (props.event.target as Element).closest('.mobile-project-grip');
    super({ ...props, options: { activationConstraint: grip ? { distance: 5 } : { delay: 350, tolerance: 8 } } });
  }
}
// Only accept a visible folder under the finger, including when the project
// list is scrolled or a target is clipped by the edge of the drawer.
const targetAtPointer: CollisionDetection = ({ pointerCoordinates, droppableContainers }) => {
  if (!pointerCoordinates) return [];
  const button = dropButtonAt(pointerCoordinates.x, pointerCoordinates.y);
  const target = button && droppableContainers.find(container => container.node.current === button);
  return target ? [{ id: target.id }] : [];
};

function ProjectDestination({ id, name, selected, disabled, onSelect }: {
  id: string | null; name: string; selected: boolean; disabled: boolean; onSelect(): void;
}) {
  const { setNodeRef, isOver } = useDroppable({ id: 'project:' + (id ?? ''), data: { projectId: id }, disabled: selected || disabled });
  const Icon = !id ? MessagesSquare : selected ? FolderOpen : Folder;
  return <button ref={setNodeRef} className="mobile-project-destination" data-project-drop={id ?? ''} data-over={isOver || undefined}
    aria-pressed={selected} disabled={disabled} onClick={onSelect}>
    <Icon size={17} aria-hidden="true"/><span>{name}</span>{selected && <Check size={14} aria-hidden="true"/>}
  </button>;
}

type Editor = { kind: 'create' } | { kind: 'move'; chat: Chat } | { kind: 'menu' | 'rename' | 'delete'; project: Project };

export function MobileProjectHistory({ projects, conversations, activeId, archiveId, onArchive, onOpen, hasMore, onMore }: {
  projects?: MobileProjectView; conversations: Chat[]; activeId: string; archiveId?: string;
  onArchive?: (chat: Chat) => void; onOpen(chat: Chat): void; hasMore: boolean; onMore(): void;
}) {
  const { t } = useTranslation();
  const [editor, setEditor] = useState<Editor | null>(null);
  const [name, setName] = useState('');
  const [failure, setFailure] = useState('');
  const [busy, setBusy] = useState(false);
  const locked = useRef(false);
  const [lifted, setLifted] = useState<Chat | null>(null);
  const [lastMove, setLastMove] = useState<string | null>(null);
  const releasedAt = useRef<{ clientX: number; clientY: number } | null>(null);
  // When the browser coalesces a fast drag into one move, that move can
  // activate the sensor before it measures the new target. Use the actual
  // release coordinates for the drop, including drops outside the list.
  useEffect(() => {
    const mouseUp = ({ clientX, clientY }: MouseEvent) => { releasedAt.current = { clientX, clientY }; };
    const touchEnd = (event: TouchEvent) => {
      const touch = event.changedTouches[0];
      releasedAt.current = touch ? { clientX: touch.clientX, clientY: touch.clientY } : null;
    };
    window.addEventListener('mouseup', mouseUp, true);
    window.addEventListener('touchend', touchEnd, true);
    return () => {
      window.removeEventListener('mouseup', mouseUp, true);
      window.removeEventListener('touchend', touchEnd, true);
    };
  }, []);
  const sensors = useSensors(useSensor(ChatMouseSensor), useSensor(ChatTouchSensor));
  const selectedId = projects?.selectedId ?? null;
  const selected = projects?.projects.find(project => project.id === selectedId);
  const projectList = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const list = projectList.current;
    const button = list?.querySelector('[aria-pressed="true"]');
    if (!list || !button) return;
    const bounds = list.getBoundingClientRect();
    const row = button.getBoundingClientRect();
    if (row.top < bounds.top) list.scrollTop += row.top - bounds.top;
    else if (row.bottom > bounds.bottom) list.scrollTop += row.bottom - bounds.bottom;
  }, [selectedId, projects?.projects]);
  const chats = conversations.filter(chat => (projects?.memberships[chat.id] ?? null) === selectedId);
  const editable = !!projects && (!projects.loading || projects.projects.length > 0) && !busy;

  const show = (next: Editor) => {
    setName(next.kind === 'rename' ? next.project.name : '');
    setFailure('');
    setEditor(next);
  };
  const save = async (work: () => Promise<void>) => {
    if (locked.current) return;
    locked.current = true; setBusy(true); setFailure('');
    try { await work(); setEditor(null); }
    catch (error) { setFailure(error instanceof Error ? error.message : String(error)); }
    finally { locked.current = false; setBusy(false); }
  };
  const move = (chat: Chat, target: string | null) => {
    if (!projects || (projects.memberships[chat.id] ?? null) === target) return;
    setLastMove(chat.id);
    void save(() => projects.move(chat.id, target));
  };
  const form = editor?.kind === 'create' || editor?.kind === 'rename';
  const dialogTitle = editor?.kind === 'move' ? t('project.moveToProject') : editor?.kind === 'delete' ? t('project.delete')
    : editor?.kind === 'rename' ? t('project.editProject') : editor?.kind === 'menu' ? editor.project.name : t('project.createNew');
  const error = failure || projects?.error;
  return <>
    <DndContext sensors={sensors} collisionDetection={targetAtPointer}
      onDragStart={({ active }) => { releasedAt.current = null; setLifted(chats.find(chat => chat.id === active.id) ?? null); }}
      onDragCancel={() => setLifted(null)}
      onDragEnd={({ over, active }) => {
        setLifted(null);
        const chat = chats.find(item => item.id === active.id);
        const released = releasedAt.current;
        const destination = released ? dropButtonAt(released.clientX, released.clientY)?.dataset.projectDrop
          : over?.data.current?.projectId;
        if (chat && editable && destination !== undefined) move(chat, destination || null);
      }}>
      {projects && <section className="mobile-projects" aria-label={t('remote.chatProjects')}>
        <div className="mobile-projects-heading">
          <h3>{t('remote.chatProjects')}</h3>
          <button className="mobile-icon-button" aria-label={t('project.createNew')} title={t('project.createNew')}
            disabled={!editable} onClick={() => show({ kind:'create' })}><Plus size={18}/></button>
        </div>
        <div className="mobile-project-list" ref={projectList}>
          {projects.projects.map(project => <div className="mobile-project-entry" key={project.id}>
            <ProjectDestination id={project.id} name={project.name} selected={selectedId === project.id} disabled={busy}
              onSelect={() => projects.select(project.id)}/>
            {selectedId === project.id && <button className="mobile-icon-button" disabled={!editable}
              aria-label={t('common.moreOptions')} onClick={() => show({ kind:'menu', project })}><MoreHorizontal size={17}/></button>}
          </div>)}
        </div>
        <ProjectDestination id={null} name={t('remote.ungroupedChats')} selected={!selectedId} disabled={busy} onSelect={() => projects.select(null)}/>
      </section>}
      {!editor && error && <div className="mobile-error" role="alert">{error}{projects?.retry && <button onClick={() => { setFailure(''); projects.retry?.(); }}>{t('common.retry')}</button>}</div>}
      <nav key={selectedId ?? 'root'} className="mobile-history" aria-label={selected?.name ?? t('remote.conversation')} aria-busy={projects?.loading}>
        {projects?.loading && <p role="status" className="mobile-section-hint">{t('common.loading')}</p>}
        {selected && !projects?.loading && !error && !chats.length && <p className="mobile-project-empty">{t('chat.noConversations')}</p>}
        <AnimatePresence initial={false} custom={lastMove}>{chats.map(chat => {
          const title = chat.title || t('remote.newChat');
          return <MobileConversationRow key={chat.id} id={chat.id} title={title} preview={archiveId === chat.id}
            onArchive={onArchive && (() => { setLastMove(null); onArchive(chat); })}
            onMove={editable && projects.projects.length ? () => show({ kind:'move', chat }) : undefined}>
            <button className={chat.id === activeId ? 'selected' : ''} onClick={() => onOpen(chat)}>{title}</button>
          </MobileConversationRow>;
        })}</AnimatePresence>
      </nav>
      {hasMore && !selectedId && <button onClick={onMore}>{t('remote.moreChats')}</button>}
      <DragOverlay dropAnimation={null} style={{ pointerEvents:'none' }}>{lifted && <div className="mobile-project-drag-preview">
        <MessageSquare size={16}/><span>{lifted.title || t('remote.newChat')}</span>
      </div>}</DragOverlay>
    </DndContext>
    {projects && <MobileDialog open={!!editor} title={dialogTitle} onClose={() => { if (!busy) setEditor(null); }}>
      {error && <p className="mobile-error" role="alert">{error}</p>}
      {editor && form && <form className="mobile-settings-form" onSubmit={event => {
        event.preventDefault();
        const trimmed = name.trim();
        if (!trimmed || [...trimmed].length > 100) return;
        void save(() => editor.kind === 'rename' ? projects.rename(editor.project.id, trimmed) : projects.create(trimmed));
      }}>
        <label>{t('project.name')}<input autoFocus required value={name} disabled={busy} onChange={event => setName(event.target.value)}/></label>
        <div className="mobile-confirm-actions"><button type="button" disabled={busy} onClick={() => setEditor(null)}>{t('common.cancel')}</button>
          <button type="submit" className="mobile-primary" disabled={busy || !name.trim() || [...name.trim()].length > 100}>{t(editor.kind === 'create' ? 'common.create' : 'common.save')}</button>
        </div>
      </form>}
      {editor?.kind === 'menu' && <div className="mobile-project-options">
        <button onClick={() => show({ kind:'rename', project:editor.project })}><Pencil size={16}/>{t('project.editProject')}</button>
        <button onClick={() => show({ kind:'delete', project:editor.project })}><Trash2 size={16}/>{t('project.delete')}</button>
      </div>}
      {editor?.kind === 'delete' && <><p>{editor.project.name}</p><p className="mobile-section-hint">{t('project.deleteConfirmDesc')}</p>
        <div className="mobile-confirm-actions"><button disabled={busy} onClick={() => setEditor(null)}>{t('common.cancel')}</button>
          <button className="mobile-primary" disabled={busy} onClick={() => void save(() => projects.remove(editor.project.id))}>{t('common.delete')}</button></div>
      </>}
      {editor?.kind === 'move' && <div className="mobile-project-options">
        {projects.projects.map(project => <button key={project.id} disabled={busy || projects.memberships[editor.chat.id] === project.id}
          onClick={() => move(editor.chat, project.id)}><Folder size={17}/>{project.name}</button>)}
        {projects.memberships[editor.chat.id] && <button disabled={busy} onClick={() => move(editor.chat, null)}><MessagesSquare size={17}/>{t('project.removeFromProject')}</button>}
      </div>}
    </MobileDialog>}
  </>;
}
