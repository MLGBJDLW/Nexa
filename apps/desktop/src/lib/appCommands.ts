import { useEffect, useRef, useSyncExternalStore } from 'react';
import type { TranslationKey } from '../i18n';

export interface AppCommand {
  id: string;
  label: TranslationKey;
  keywords: string;
  enabled?: boolean;
  run: () => void;
}

const commands = new Map<string, AppCommand>();
const listeners = new Set<() => void>();
let snapshot: AppCommand[] = [];
function publish() {
  snapshot = Array.from(commands.values());
  listeners.forEach(listener => listener());
}
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
};
export const useAppCommands = () => useSyncExternalStore(subscribe, () => snapshot);

/** Commands exist only while their owning screen is mounted; callbacks stay current. */
export function useAppCommand(command: AppCommand) {
  const latest = useRef(command.run);
  latest.current = command.run;
  const { id, label, keywords, enabled = true } = command;
  useEffect(() => {
    const entry = { id, label, keywords, enabled, run: () => latest.current() };
    commands.set(id, entry);
    publish();
    return () => {
      if (commands.get(id) === entry) {
        commands.delete(id);
        publish();
      }
    };
  }, [id, label, keywords, enabled]);
}

export function runAppCommand(id: string): boolean {
  const command = commands.get(id);
  if (!command || command.enabled === false) return false;
  command.run();
  return true;
}

export const OPEN_COMMAND_PALETTE = 'nexa:open-command-palette';
export function openCommandPalette() {
  window.dispatchEvent(new Event(OPEN_COMMAND_PALETTE));
}
