import { useEffect, useRef, useState } from 'react';
import { Command, FolderOpen, Globe2, MessageSquarePlus, TerminalSquare } from 'lucide-react';
import { useLocation, useNavigate } from 'react-router';
import { useTranslation, type TranslationKey } from '../i18n';
import { openCommandPalette, runAppCommand, useAppCommands } from '../lib/appCommands';
import { NexaMenu, NexaMenuContent, NexaMenuItem, NexaMenuTrigger } from './ui/overlay/Menu';
import { Logo } from './Logo';
import './workspaceRadialMenu.css';

const SIZE = 300;
const EXCLUDED = 'input,textarea,select,button,a,summary,[role="button"],[role="textbox"],[role="menu"],[role="dialog"],[contenteditable="true"],[data-radix-collection-item],pre,code,p,li,h1,h2,h3,h4,h5,h6,blockquote,table,img,svg,canvas,video,audio,iframe,.xterm,.monaco-editor';
const ACTIONS = [
  { id:'sources', label:'nav.sources', icon:FolderOpen },
  { id:'browser', label:'shortcuts.toggleBrowser', icon:Globe2 },
  { id:'new', label:'chat.newChat', icon:MessageSquarePlus },
  { id:'terminal', label:'shortcuts.toggleTerminal', icon:TerminalSquare },
  { id:'commands', label:'nav.commandPalette', icon:Command },
] satisfies { id: string; label: TranslationKey; icon: typeof Command }[];

function point(radius: number, degrees: number) {
  const angle = degrees * Math.PI / 180;
  return [150 + radius * Math.cos(angle), 150 + radius * Math.sin(angle)];
}
const SECTORS = ACTIONS.map((_, index) => {
  const start = -126 + index * 72 + .55, end = start + 70.9;
  const [a,b,c,d] = [point(141,start),point(141,end),point(66,end),point(66,start)];
  const path = `M${a} A141,141 0 0 1 ${b} L${c} A66,66 0 0 0 ${d} Z`;
  const polygon = [...Array.from({length:17},(_,i)=>point(141,start+(end-start)*i/16)), ...Array.from({length:17},(_,i)=>point(66,end-(end-start)*i/16))].map(([x,y])=>`${x/3}% ${y/3}%`).join(',');
  const [x,y] = point(104, (start+end)/2);
  return { path, clipPath:`polygon(${polygon})`, left:`${x/3}%`, top:`${y/3}%` };
});

function hitsText(event: MouseEvent) {
  const range = document.caretRangeFromPoint?.(event.clientX,event.clientY);
  if (!range || range.startContainer.nodeType !== Node.TEXT_NODE) return false;
  const length = range.startContainer.textContent?.length ?? 0;
  if (!length) return false;
  const index = Math.min(range.startOffset,length-1);
  range.setStart(range.startContainer,index); range.setEnd(range.startContainer,index+1);
  return Array.from(range.getClientRects()).some(rect => event.clientX >= rect.left && event.clientX <= rect.right && event.clientY >= rect.top && event.clientY <= rect.bottom);
}

/** Blank workspace only. Native editing and feature-specific menus keep ownership. */
export function WorkspaceRadialMenu() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const commands = useAppCommands();
  const [open,setOpen] = useState(false);
  const [anchor,setAnchor] = useState({x:150,y:150});
  const [active,setActive] = useState<number | null>(null);
  const previousFocus = useRef<HTMLElement | null>(null);
  const restoreFocus = useRef(true);
  const content = useRef<HTMLDivElement | null>(null);
  useEffect(() => { setOpen(false); }, [location.key]);
  useEffect(() => {
    const context = (event: MouseEvent) => {
      const target = event.target;
      if (event.defaultPrevented || !(target instanceof Element) || !target.closest('[data-workspace-context]') || target.closest(EXCLUDED) || window.getSelection()?.toString() || hitsText(event)) return;
      if (document.querySelector('[role="dialog"][data-state="open"],dialog[open],[aria-modal="true"]')) return;
      event.preventDefault();
      previousFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      restoreFocus.current = true;
      const bounds = target.getBoundingClientRect();
      const x = event.clientX || bounds.left + bounds.width/2;
      const y = event.clientY || bounds.top + bounds.height/2;
      const radius = Math.min(SIZE, window.innerWidth-24, window.innerHeight-24)/2;
      setAnchor({x:Math.max(radius+12,Math.min(window.innerWidth-radius-12,x)),y:Math.max(radius+12,Math.min(window.innerHeight-radius-12,y))});
      setActive(null); setOpen(true);
    };
    window.addEventListener('contextmenu',context);
    return () => window.removeEventListener('contextmenu',context);
  }, []);
  useEffect(() => {
    if (!open) return;
    const close = () => setOpen(false);
    window.addEventListener('resize',close); window.addEventListener('blur',close);
    return () => { window.removeEventListener('resize',close); window.removeEventListener('blur',close); };
  }, [open]);
  const run = (id: string) => {
    restoreFocus.current = false; setOpen(false);
    requestAnimationFrame(() => {
      if (id === 'sources') navigate('/sources');
      else if (id === 'commands') openCommandPalette();
      else if (!runAppCommand(`chat.${id}`)) navigate('/chat', { state: { pendingChatAction: id } });
    });
  };
  return <NexaMenu open={open} onOpenChange={setOpen} modal={false}>
    <NexaMenuTrigger asChild><button tabIndex={-1} aria-hidden="true" className="workspace-radial-anchor" style={{left:anchor.x,top:anchor.y}}/></NexaMenuTrigger>
    <NexaMenuContent ref={content} className="workspace-radial-menu" align="center" side="bottom" sideOffset={-SIZE/2} collisionPadding={12}
      aria-label={t('nav.commandPalette')} data-testid="workspace-radial-menu"
      onInteractOutside={() => { restoreFocus.current = false; }}
      onCloseAutoFocus={event => { event.preventDefault(); if (restoreFocus.current && previousFocus.current?.isConnected) previousFocus.current.focus({preventScroll:true}); }}
      onKeyDown={event => {
        if (!['ArrowLeft','ArrowRight'].includes(event.key)) return;
        event.preventDefault();
        const items=Array.from(content.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not([data-disabled])') ?? []);
        const index=items.indexOf(document.activeElement as HTMLElement);
        items[(index + (event.key==='ArrowRight'?1:-1) + items.length) % items.length]?.focus();
      }}>
      <div className="workspace-radial-face" aria-hidden="true"/>
      {ACTIONS.map((action,index) => {
        const command = commands.find(item=>item.id===`chat.${action.id}`);
        const Icon = action.icon, sector=SECTORS[index];
        return <NexaMenuItem key={action.id} className="workspace-radial-sector" textValue={t(action.label)} aria-label={t(action.label)}
          disabled={command?.enabled===false} style={{clipPath:sector.clipPath}} onPointerMove={()=>setActive(index)} onFocus={()=>setActive(index)} onSelect={()=>run(action.id)}>
          <svg viewBox="0 0 300 300" aria-hidden="true" className="workspace-radial-shape"><path d={sector.path}/></svg>
          <span className="workspace-radial-icon" style={{left:sector.left,top:sector.top}}><Icon size={22} strokeWidth={1.65}/></span>
        </NexaMenuItem>;
      })}
      <button className="workspace-radial-hub" tabIndex={-1} aria-label={t('common.close')} onClick={()=>setOpen(false)}>
        <Logo size={36} decorative />
        <span>{active === null ? 'Nexa' : t(ACTIONS[active].label)}</span>
      </button>
    </NexaMenuContent>
  </NexaMenu>;
}
