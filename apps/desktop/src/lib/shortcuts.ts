import type { TranslationKey } from '../i18n/types';
import { useSyncExternalStore } from 'react';

export interface Shortcut {
  keys: string;
  macKeys: string;
  description: TranslationKey;
  scope: 'global' | 'chat' | 'search';
}

const isMac =
  typeof navigator !== 'undefined' &&
  /Mac|iPod|iPhone|iPad/.test(navigator.platform);

export const SHORTCUTS: Shortcut[] = [
  { keys: 'Ctrl+Shift+P', macKeys: '⌘⇧P', description: 'shortcuts.commandPalette', scope: 'global' },
  { keys: 'Ctrl+B', macKeys: '⌘B', description: 'shortcuts.toggleSidebar', scope: 'chat' },
  { keys: 'Ctrl+Shift+B', macKeys: '⌘⇧B', description: 'shortcuts.toggleBrowser', scope: 'chat' },
  { keys: 'Ctrl+J', macKeys: '⌘J', description: 'shortcuts.toggleTerminal', scope: 'chat' },
  { keys: 'Ctrl+Shift+A', macKeys: '⌘⇧A', description: 'shortcuts.askAi', scope: 'search' },
];

/** Return the platform-appropriate key display string. */
export function formatKeys(shortcut: Shortcut): string {
  return isMac ? shortcut.macKeys : shortcut.keys;
}

const PALETTE_SHORTCUT_KEY = 'nexa-command-palette-shortcut';
export const DEFAULT_PALETTE_SHORTCUT = 'Mod+Shift+P';
const RESERVED_SHORTCUTS = new Set(['Mod+Shift+B', 'Mod+Shift+A']);
export function validPaletteShortcut(value: string): boolean {
  // Require a modified chord so ordinary typing, editing, and chat hotkeys stay intact.
  return /^Mod\+(?:Alt\+(?:Shift\+)?|Shift\+)[A-Z0-9]$/.test(value) && !RESERVED_SHORTCUTS.has(value);
}
function readPaletteShortcut() {
  try {
    const value = localStorage.getItem(PALETTE_SHORTCUT_KEY) ?? '';
    return validPaletteShortcut(value) ? value : DEFAULT_PALETTE_SHORTCUT;
  } catch { return DEFAULT_PALETTE_SHORTCUT; }
}
let paletteShortcut = readPaletteShortcut();
const paletteListeners = new Set<() => void>();
const subscribePaletteShortcut = (listener: () => void) => {
  paletteListeners.add(listener);
  return () => { paletteListeners.delete(listener); };
};
export const usePaletteShortcut = () => useSyncExternalStore(subscribePaletteShortcut, () => paletteShortcut);
export function setPaletteShortcut(value: string) {
  if (!validPaletteShortcut(value)) return false;
  paletteShortcut = value;
  try { localStorage.setItem(PALETTE_SHORTCUT_KEY, value); } catch { /* Session preference still works. */ }
  paletteListeners.forEach(listener => listener());
  return true;
}
if (typeof window !== 'undefined') window.addEventListener('storage', event => {
  if (event.key !== PALETTE_SHORTCUT_KEY && event.key !== null) return;
  paletteShortcut = readPaletteShortcut();
  paletteListeners.forEach(listener => listener());
});
export function shortcutFromEvent(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'shiftKey' | 'altKey'>): string {
  if (!event.ctrlKey && !event.metaKey) return '';
  return `Mod+${event.altKey ? 'Alt+' : ''}${event.shiftKey ? 'Shift+' : ''}${event.key.toUpperCase()}`;
}
export function formatPaletteShortcut(value: string): string {
  return value.replace('Mod', isMac ? '⌘' : 'Ctrl');
}
