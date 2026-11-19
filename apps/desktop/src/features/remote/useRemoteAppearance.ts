import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from '../../i18n';
import type { AppearanceRegistry } from '../../lib/api';
import { applyTheme, getInitialTheme, isThemeId } from '../../lib/theme';
import { applyCustomTheme, clearCustomThemeVariables, normalizeThemeResourcePlugin, themeResourcePluginToCustomTheme } from '../../lib/themeProfile';
import type { RemoteChatClient } from './remoteClient';

interface RemotePreferences { locale: string; appearance: AppearanceRegistry }
export function useRemoteAppearance(client: RemoteChatClient | null, followLanguage = true, localAppearance?: AppearanceRegistry) {
  const { setLocale, availableLocales } = useTranslation();
  const [preferences, setPreferences] = useState<(RemotePreferences & { serverId: string }) | null>(null);
  const serverId = client?.paired.manifest.serverId ?? 'unpaired';
  const currentPreferences = preferences?.serverId === serverId ? preferences : null;
  const key = `nexa.remote.preferences.${serverId}`;
  const [choice, setChoice] = useState<{ theme: string; language: string }>({ theme:'desktop', language:'desktop' });
  const [error, setError] = useState(false);
  const choices = useRef(choice); choices.current = choice;
  useEffect(() => {
    try { setChoice({ theme:'desktop', language:'desktop', ...JSON.parse(localStorage.getItem(key) || '{}') }); }
    catch { setChoice({ theme:'desktop', language:'desktop' }); }
    setPreferences(null);
  }, [key]);
  useEffect(() => {
    if (!client) return;
    let disposed = false;
    let pending = false;
    const refresh = async () => {
      if (pending || client.state.phase !== 'connected') return;
      pending = true;
      try {
        const next = await client.rpc<RemotePreferences>('connection.preferences');
        if (!disposed && next?.appearance && Array.isArray(next.appearance.plugins)) {
          setPreferences(current => current?.serverId === serverId && current.locale === next.locale && JSON.stringify(current.appearance) === JSON.stringify(next.appearance) ? current : { ...next, serverId });
        }
      } catch { /* Preserve the active skin across temporary route failures. */ }
      finally { pending = false; }
    };
    const off = client.subscribeConnection(() => void refresh());
    const timer = setInterval(() => { if (!document.hidden) void refresh(); }, 30_000);
    return () => { disposed = true; off(); clearInterval(timer); };
  }, [client, serverId]);
  const language = choice.language === 'desktop' ? currentPreferences?.locale : choice.language;
  useEffect(() => {
    const supported = availableLocales.find(locale => locale.code === language);
    if (followLanguage && supported) setLocale(supported.code);
  }, [availableLocales, followLanguage, language, setLocale]);
  const registry = client ? currentPreferences?.appearance : localAppearance?.revision ? localAppearance : undefined;
  const id = !client || choice.theme === 'desktop' ? registry?.activeThemeId : choice.theme;
  const plugin = registry?.plugins.find(plugin => plugin.id === id);
  const activeTheme = useMemo(() => {
    if (plugin) {
      try { return themeResourcePluginToCustomTheme(normalizeThemeResourcePlugin(plugin)); }
      catch { return undefined; }
    }
  }, [plugin]);
  useEffect(() => {
    setError(Boolean(plugin && !activeTheme));
    if (activeTheme) {
      applyTheme(activeTheme.baseTheme);
      applyCustomTheme(activeTheme);
      document.documentElement.style.colorScheme = activeTheme.mode;
    } else {
      clearCustomThemeVariables();
      applyTheme(id && isThemeId(id) ? id : getInitialTheme());
    }
  }, [id, activeTheme, plugin]);
  const resolveMedia = useCallback(async (assetId: string) => {
    if (!client) throw new Error('Theme host is unavailable');
    const url = await client.rpc<string>('appearance.background', { assetId });
    const match = /^data:(image\/(?:png|jpeg|webp|gif)|video\/(?:mp4|webm));base64,/.exec(url);
    if (!match) throw new Error('Unsupported managed background media');
    return { mediaType: match[1], url };
  }, [client]);
  const reportUnavailable = useCallback(() => setError(true), []);
  const update = (patch: Partial<typeof choice>) => {
    const next = { ...choices.current, ...patch };
    choices.current = next; setChoice(next);
    localStorage.setItem(key, JSON.stringify(next));
  };
  return { ...choice, customThemes:currentPreferences?.appearance.plugins ?? [], themed: Boolean(registry?.revision), activeTheme, resolveMedia, reportUnavailable, error, update };
}
