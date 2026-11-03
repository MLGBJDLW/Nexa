import { useEffect, useRef, type ReactNode } from 'react';
import { Menu, SquarePen, X } from 'lucide-react';
import { useTranslation } from '../../i18n';

export function MobileChatHeader({ center, onMenu, onNew, disabled }: { center: ReactNode; onMenu: () => void; onNew?: () => void; disabled?: boolean }) {
  const { t } = useTranslation();
  return <header className="native-chat-header"><button className="mobile-icon-button" onClick={onMenu} aria-label={t('remote.conversation')}><Menu size={21}/></button><div className="native-chat-heading">{center}</div>{onNew ? <button className="mobile-icon-button" onClick={onNew} disabled={disabled} aria-label={t('remote.newChat')}><SquarePen size={21}/></button> : <span className="mobile-icon-spacer"/>}</header>;
}

export function MobileDialog({ title, side = false, children, onClose }: { title: string; side?: boolean; children: ReactNode; onClose: () => void }) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => { const node = dialog.current!; node.showModal(); return () => node.close(); }, []);
  return <dialog ref={dialog} className={side ? 'mobile-dialog mobile-drawer' : 'mobile-dialog mobile-sheet'} aria-label={title} onCancel={event => { event.preventDefault(); onClose(); }} onClick={event => { if (event.target === event.currentTarget) onClose(); }}><div className="mobile-dialog-body"><header><h2>{title}</h2><button className="mobile-icon-button" onClick={onClose} aria-label={t('common.close')}><X size={20}/></button></header>{children}</div></dialog>;
}

export interface MobileChatPresentation {
  heading: ReactNode;
  drawerFooter: ReactNode;
  caption: string;
  onConfigure: () => void;
  onFiles: () => void;
}
