import { useEffect, useRef, useState } from 'react';
import { useTranslation } from '../../i18n';
import type { AppearanceRegistry } from '../../lib/api';
import { applyTheme, getInitialTheme, isThemeId } from '../../lib/theme';
import { applyCustomTheme, clearCustomThemeVariables, normalizeThemeResourcePlugin, themeResourcePluginToCustomTheme } from '../../lib/themeProfile';
import type { RemoteClient } from './remoteClient';

interface RemotePreferences { locale: string; appearance: AppearanceRegistry }
export function useRemoteAppearance(client: RemoteClient | null) {
  const { setLocale, availableLocales } = useTranslation();
  const [preferences, setPreferences] = useState<RemotePreferences | null>(null);
  const serverId = client?.paired.manifest.serverId ?? 'unpaired';
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
          setPreferences(current => current?.locale === next.locale && JSON.stringify(current.appearance) === JSON.stringify(next.appearance) ? current : next);
        }
      } catch { /* Preserve the active skin across temporary route failures. */ }
      finally { pending = false; }
    };
    const off = client.subscribeConnection(() => void refresh());
    const timer = setInterval(() => { if (!document.hidden) void refresh(); }, 30_000);
    return () => { disposed = true; off(); clearInterval(timer); };
  }, [client]);
  const language = choice.language === 'desktop' ? preferences?.locale : choice.language;
  useEffect(() => {
    const supported = availableLocales.find(locale => locale.code === language);
    if (supported) setLocale(supported.code);
  }, [language, setLocale]);
  useEffect(() => {
    let disposed = false;
    setError(false);
    const id = choice.theme === 'desktop' ? preferences?.appearance.activeThemeId : choice.theme;
    const plugin = preferences?.appearance.plugins.find(plugin => plugin.id === id);
    if (plugin) {
      try {
        const theme = themeResourcePluginToCustomTheme(normalizeThemeResourcePlugin(plugin));
        applyTheme(theme.baseTheme);
        applyCustomTheme(theme);
        document.documentElement.style.colorScheme = theme.mode;
        if (theme.background.kind === 'image' && theme.background.assetId && client) {
          void client.rpc<string>('appearance.background', { assetId:theme.background.assetId }).then(url => {
            if (!disposed && /^data:image\/(png|jpeg|webp|gif);base64,/.test(url)) applyCustomTheme(theme, url);
          }).catch(() => { if (!disposed) setError(true); });
        }
      } catch { clearCustomThemeVariables(); applyTheme(getInitialTheme()); setError(true); }
    } else { clearCustomThemeVariables(); applyTheme(id && isThemeId(id) ? id : getInitialTheme()); }
    return () => { disposed = true; };
  }, [client, choice.theme, preferences?.appearance]);
  const update = (patch: Partial<typeof choice>) => {
    const next = { ...choices.current, ...patch };
    choices.current = next; setChoice(next);
    localStorage.setItem(key, JSON.stringify(next));
  };
  return { ...choice, customThemes:preferences?.appearance.plugins ?? [], error, update };
}
