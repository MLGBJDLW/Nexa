import { useEffect, useRef, useState } from 'react';
import { Command } from 'cmdk';
import { useLocation, useNavigate } from 'react-router';
import { motion, AnimatePresence, useReducedMotion } from 'framer-motion';
import { Search, FolderOpen, MessageCircle, Settings, ScanSearch, Database, Clock, Keyboard, Archive } from 'lucide-react';
import * as api from '../lib/api';
import type { QueryLog } from '../types';
import { useTranslation } from '../i18n';
import { formatPaletteShortcut, shortcutFromEvent, usePaletteShortcut } from '../lib/shortcuts';
import { OPEN_COMMAND_PALETTE, useAppCommands } from '../lib/appCommands';
import type { Conversation } from '../types/conversation';
import { Modal } from './ui/Modal';
import { KeyboardShortcutsSettings } from './settings/KeyboardShortcutsSettings';
import { getSoftDropdownMotion, INSTANT_TRANSITION } from '../lib/uiMotion';

type BatchAction = 'scanAll' | 'rebuildEmbeddings';

type CloseReason = 'dismiss' | 'outside';

const FOCUSABLE_SELECTOR = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(', ');

function getFocusableElements(container: HTMLElement) {
  return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
    (element) => !element.hasAttribute('disabled') && element.getAttribute('aria-hidden') !== 'true',
  );
}

function activeConversationIdFromPath(pathname: string): string | null {
  const match = pathname.match(/^\/chat\/([^/?#]+)/);
  if (!match) return null;
  try {
    return decodeURIComponent(match[1]);
  } catch {
    return match[1];
  }
}

export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [recentQueries, setRecentQueries] = useState<QueryLog[]>([]);
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [query, setQuery] = useState('');
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const shortcut = usePaletteShortcut();
  const commands = useAppCommands();
  const dialogRef = useRef<HTMLDivElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  const shouldRestoreFocusRef = useRef(false);
  const navigate = useNavigate();
  const location = useLocation();
  const { t } = useTranslation();
  const shouldReduceMotion = useReducedMotion();
  const activeConversationId = activeConversationIdFromPath(location.pathname);

  const closePalette = (reason: CloseReason = 'dismiss') => {
    shouldRestoreFocusRef.current = reason !== 'outside';
    setOpen(false);
  };

  /* Both the configurable chord and the familiar Ctrl/Cmd+K open the same palette. */
  useEffect(() => {
    const toggle = () => {
        if (open) {
          closePalette();
          return;
        }

        const activeElement = document.activeElement;
        restoreFocusRef.current = activeElement instanceof HTMLElement ? activeElement : null;
        shouldRestoreFocusRef.current = true;
        setQuery('');
        setOpen(true);
    };
    const handler = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing || e.repeat) return;
      const chord = shortcutFromEvent(e);
      if (chord !== shortcut && chord !== 'Mod+K') return;
      e.preventDefault();
      toggle();
    };
    document.addEventListener('keydown', handler);
    window.addEventListener(OPEN_COMMAND_PALETTE, toggle);
    return () => {
      document.removeEventListener('keydown', handler);
      window.removeEventListener(OPEN_COMMAND_PALETTE, toggle);
    };
  }, [open, shortcut]);

  /* ── Escape to close ─────────────────────────────────────────────── */
  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        closePalette();
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [open]);

  /* ── Capture focus entry point and restore on close ──────────────── */
  useEffect(() => {
    if (!open) {
      if (shouldRestoreFocusRef.current && restoreFocusRef.current?.isConnected) {
        restoreFocusRef.current.focus();
      }
      shouldRestoreFocusRef.current = false;
      restoreFocusRef.current = null;
      return;
    }

    if (!restoreFocusRef.current) {
      const activeElement = document.activeElement;
      restoreFocusRef.current = activeElement instanceof HTMLElement ? activeElement : null;
      shouldRestoreFocusRef.current = true;
    }
  }, [open]);

  /* ── Trap focus while modal is open ──────────────────────────────── */
  useEffect(() => {
    if (!open) return;

    const trapFocus = (event: KeyboardEvent) => {
      if (event.key !== 'Tab') return;

      const container = dialogRef.current;
      if (!container) return;

      const focusableElements = getFocusableElements(container);
      if (focusableElements.length === 0) {
        event.preventDefault();
        container.focus();
        return;
      }

      const firstElement = focusableElements[0];
      const lastElement = focusableElements[focusableElements.length - 1];
      const activeElement = document.activeElement instanceof HTMLElement ? document.activeElement : null;

      if (!activeElement || !container.contains(activeElement)) {
        event.preventDefault();
        (event.shiftKey ? lastElement : firstElement).focus();
        return;
      }

      if (!event.shiftKey && activeElement === lastElement) {
        event.preventDefault();
        firstElement.focus();
      }

      if (event.shiftKey && activeElement === firstElement) {
        event.preventDefault();
        lastElement.focus();
      }
    };

    const keepFocusInside = (event: FocusEvent) => {
      const container = dialogRef.current;
      const target = event.target;
      if (!container || !(target instanceof Node) || container.contains(target)) {
        return;
      }

      const focusableElements = getFocusableElements(container);
      (focusableElements[0] ?? container).focus();
    };

    document.addEventListener('keydown', trapFocus);
    document.addEventListener('focusin', keepFocusInside);

    return () => {
      document.removeEventListener('keydown', trapFocus);
      document.removeEventListener('focusin', keepFocusInside);
    };
  }, [open]);

  /* ── Load recent queries on open ─────────────────────────────────── */
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setConversations([]);
    setRecentQueries([]);
    void api.getRecentQueries(5).then(rows => { if (!cancelled) setRecentQueries(rows ?? []); }).catch(() => {});
    void api.listConversations().then(rows => { if (!cancelled) setConversations(rows ?? []); }).catch(() => {});
    return () => { cancelled = true; };
  }, [open]);

  /* ── Helpers ─────────────────────────────────────────────────────── */
  const select = (fn: () => void) => {
    closePalette();
    // Let the modal release its focus trap before opening the selected control.
    requestAnimationFrame(fn);
  };

  const openBatchActionConfirmation = (action: BatchAction) => {
    select(() => {
      navigate('/sources', { state: { pendingBatchAction: action } });
    });
  };

  const compactActiveConversation = () => {
    if (!activeConversationId) return;
    select(() => {
      navigate(`/chat/${activeConversationId}`, {
        state: { pendingChatAction: 'compact' },
      });
    });
  };

  /* ── Render ──────────────────────────────────────────────────────── */
  return (<>
    <AnimatePresence>
      {open && (
        <div className="fixed inset-0 z-50">
          {/* Backdrop */}
          <motion.div
            className="absolute inset-0 bg-black/60 backdrop-blur-sm"
            aria-hidden="true"
            initial={shouldReduceMotion ? false : { opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={shouldReduceMotion ? INSTANT_TRANSITION : { duration: 0.15 }}
            onClick={() => closePalette('outside')}
          />

          {/* Dialog */}
          <motion.div
            ref={dialogRef}
            className="absolute left-1/2 top-[14%] w-full max-w-xl -translate-x-1/2 px-4"
            role="dialog"
            aria-modal="true"
            aria-label={t('nav.commandPalette')}
            tabIndex={-1}
            {...getSoftDropdownMotion(!!shouldReduceMotion, -8)}
          >
            <Command
              className="overflow-hidden rounded-xl border border-border bg-surface-1 shadow-lg"
              loop
            >
              <Command.Input
                value={query}
                onValueChange={setQuery}
                placeholder={t('cmd.placeholder')}
                aria-label={t('cmd.placeholder')}
                className="w-full border-b border-border bg-transparent px-4 py-3 text-sm
                  text-text-primary placeholder:text-text-tertiary outline-none"
                autoFocus
              />

              <Command.List className="max-h-[min(60vh,28rem)] overflow-y-auto p-2">
                <Command.Empty className="px-4 py-8 text-center text-sm text-text-tertiary">
                  {t('cmd.noResults')}
                </Command.Empty>

                {/* Navigation */}
                <Command.Group heading={t('cmd.navigation')}>
                  <Command.Item onSelect={() => select(() => navigate('/'))}>
                    <Search className="h-4 w-4 shrink-0 text-text-tertiary" />
                    {t('nav.search')}
                  </Command.Item>
                  <Command.Item onSelect={() => select(() => navigate('/sources'))}>
                    <FolderOpen className="h-4 w-4 shrink-0 text-text-tertiary" />
                    {t('nav.sources')}
                  </Command.Item>
                  <Command.Item onSelect={() => select(() => navigate('/chat'))}>
                    <MessageCircle className="h-4 w-4 shrink-0 text-text-tertiary" />
                    <span className="flex-1">{t('nav.chat')}</span>
                  </Command.Item>
                  <Command.Item onSelect={() => select(() => navigate('/settings'))}>
                    <Settings className="h-4 w-4 shrink-0 text-text-tertiary" />
                    {t('nav.settings')}
                  </Command.Item>
                  {(['tasks', 'workflows', 'knowledge'] as const).map(route => <Command.Item key={route} value={`navigate ${route} ${t(`nav.${route}`)}`} onSelect={() => select(() => navigate(`/${route}`))}><FolderOpen className="h-4 w-4 shrink-0 text-text-tertiary" />{t(`nav.${route}`)}</Command.Item>)}
                </Command.Group>

                {commands.length > 0 && <Command.Group heading={t('cmd.currentChat')}>
                  {commands.map(command => <Command.Item key={command.id} value={`${command.id} ${t(command.label)} ${command.keywords}`} disabled={command.enabled === false} onSelect={() => select(command.run)}>
                    <MessageCircle className="h-4 w-4 shrink-0 text-text-tertiary" /><span>{t(command.label)}</span>
                  </Command.Item>)}
                </Command.Group>}

                <Command.Separator className="mx-2 my-1 h-px bg-border" />

                {/* Actions */}
                <Command.Group heading={t('cmd.actions')}>
                  <Command.Item onSelect={() => openBatchActionConfirmation('scanAll')}>
                    <ScanSearch className="h-4 w-4 shrink-0 text-text-tertiary" />
                    {t('cmd.scanAll')}
                  </Command.Item>
                  <Command.Item onSelect={() => openBatchActionConfirmation('rebuildEmbeddings')}>
                    <Database className="h-4 w-4 shrink-0 text-text-tertiary" />
                    {t('cmd.rebuildEmbeddings')}
                  </Command.Item>
                  {activeConversationId && !commands.some(command => command.id === 'chat.compact') && (
                    <Command.Item
                      value={`compact conversation context ${t('chat.compactNow')}`}
                      onSelect={compactActiveConversation}
                    >
                      <Archive className="h-4 w-4 shrink-0 text-text-tertiary" />
                      {t('chat.compactNow')}
                    </Command.Item>
                  )}
                </Command.Group>

                <Command.Group heading={t('cmd.conversations')}>
                  {conversations.filter(conversation => !conversation.archivedAt && (!query.trim() || conversation.title.toLowerCase().includes(query.trim().toLowerCase()))).slice(0, query ? 30 : 8).map(conversation => <Command.Item key={conversation.id} value={`conversation ${conversation.id} ${conversation.title}`} onSelect={() => select(() => navigate(`/chat/${encodeURIComponent(conversation.id)}`))}>
                    <MessageCircle className="h-4 w-4 shrink-0 text-text-tertiary" /><span className="truncate">{conversation.title}</span>
                  </Command.Item>)}
                </Command.Group>
                {query.trim() && <Command.Item forceMount value="search-documents" onSelect={() => select(() => navigate('/', { state: { query: query.trim() } }))}>
                  <Search className="h-4 w-4 shrink-0 text-text-tertiary" /><span className="truncate">{t('cmd.searchDocuments', { query: query.trim() })}</span>
                </Command.Item>}

                {/* Recent queries */}
                {recentQueries.length > 0 && (
                  <>
                    <Command.Separator className="mx-2 my-1 h-px bg-border" />
                    <Command.Group heading={t('cmd.recentQueries')}>
                      {recentQueries.map((q) => (
                        <Command.Item
                          key={q.id}
                          value={q.queryText}
                          onSelect={() => select(() => navigate('/', { state: { query: q.queryText } }))}
                        >
                          <Clock className="h-4 w-4 shrink-0 text-text-tertiary" />
                          <span className="truncate">{q.queryText}</span>
                        </Command.Item>
                      ))}
                    </Command.Group>
                  </>
                )}
                <Command.Separator className="mx-2 my-1 h-px bg-border" />
                <Command.Group heading={t('cmd.shortcuts')}>
                    <Command.Item value={`keyboard shortcuts hotkeys ${t('cmd.shortcuts')}`} onSelect={() => select(() => setShortcutsOpen(true))}>
                      <Keyboard className="h-4 w-4 shrink-0 text-text-tertiary" />
                      <span className="flex-1">{t('shortcuts.customize')}</span>
                      <kbd className="ml-auto rounded bg-surface-2 px-1.5 py-0.5 text-[10px] font-medium text-text-tertiary">
                        {formatPaletteShortcut(shortcut)}
                      </kbd>
                    </Command.Item>
                </Command.Group>
              </Command.List>
              <div className="border-t border-border px-4 py-2 text-[11px] text-text-tertiary">{t('cmd.keyboardHint')}</div>
            </Command>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
    <Modal open={shortcutsOpen} onClose={() => setShortcutsOpen(false)} title={t('cmd.shortcuts')}><KeyboardShortcutsSettings /></Modal>
    </>
  );
}
