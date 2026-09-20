import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { type ThemeId, getInitialTheme, applyTheme, isThemeId } from './theme';
import * as api from './api';
import {
  applyCustomTheme,
  clearCustomThemeVariables,
  normalizeThemeResourcePlugin,
  shouldApplyRegistryRevision,
  themeResourcePluginToCustomTheme,
  themeToResourcePlugin,
  type CustomThemeDefinition,
  type ThemeContent,
  type ThemeResourcePlugin,
} from './themeProfile';
import { persistStartupAppearance, snapshotStartupAppearance } from './startupAppearance';
import { connectEventSubscriptions } from './eventSubscriptions';

interface ThemeContextValue {
  theme: ThemeId;
  activeThemeId: string;
  customThemes: CustomThemeDefinition[];
  themePlugins: ThemeResourcePlugin[];
  content: ThemeContent;
  setTheme: (theme: string) => void;
  installThemePlugin: (plugin: ThemeResourcePlugin) => void;
  uninstallThemePlugin: (id: string, additionallyRetainedAssetIds?: string[]) => void;
  rollbackTheme: () => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [themePlugins, setThemePlugins] = useState<ThemeResourcePlugin[]>(readThemePlugins);
  const customThemes = useMemo(
    () => themePlugins.map(themeResourcePluginToCustomTheme),
    [themePlugins],
  );
  const [activeThemeId, setActiveThemeId] = useState<string>(() => {
    const stored = localStorage.getItem(ACTIVE_THEME_KEY);
    return stored && (isThemeId(stored) || readThemePlugins().some((plugin) => plugin.id === stored))
      ? stored
      : getInitialTheme();
  });
  const registryRevisionRef = useRef(0);
  const mutationSequenceRef = useRef(0);
  const hydrationRef = useRef<Promise<api.AppearanceRegistry> | null>(null);
  const activeCustomTheme = customThemes.find((profile) => profile.id === activeThemeId);
  const theme = activeCustomTheme?.baseTheme ?? (isThemeId(activeThemeId) ? activeThemeId : 'dark');
  const content = activeCustomTheme?.content ?? {};

  useEffect(() => {
    let cancelled = false;
    applyTheme(theme);
    if (activeCustomTheme) {
      applyCustomTheme(activeCustomTheme);
      const assetId = activeCustomTheme.background.kind === 'image'
        ? activeCustomTheme.background.assetId
        : undefined;
      if (assetId) {
        void api.resolveThemeBackground(assetId)
          .then((asset) => {
            if (!cancelled) applyCustomTheme(activeCustomTheme, convertFileSrc(asset.path));
          })
          .catch((error: unknown) => {
            console.warn('Unable to resolve managed theme background', error);
          });
      }
    } else clearCustomThemeVariables();
    persistStartupAppearance(snapshotStartupAppearance(
      activeCustomTheme?.mode ?? (theme === 'light' || theme === 'bloom' ? 'light' : 'dark'),
      activeCustomTheme?.content,
    ));
    localStorage.setItem(ACTIVE_THEME_KEY, activeThemeId);
    return () => { cancelled = true; };
  }, [activeCustomTheme, activeThemeId, theme]);

  const applyRegistry = useCallback((registry: api.AppearanceRegistry | null | undefined) => {
    if (!registry || !Array.isArray(registry.plugins)) return;
    if (!shouldApplyRegistryRevision(registryRevisionRef.current, registry.revision ?? 0)) return;
    const plugins = registry.plugins.flatMap((plugin) => {
      try { return [normalizeThemeResourcePlugin(plugin)]; } catch { return []; }
    });
    registryRevisionRef.current = registry.revision ?? 0;
    localStorage.setItem(THEME_PLUGINS_KEY, JSON.stringify(plugins));
    localStorage.setItem(ACTIVE_THEME_KEY, registry.activeThemeId);
    setThemePlugins(plugins);
    setActiveThemeId(registry.activeThemeId);
  }, []);

  const restoreLocalSnapshot = useCallback((plugins: ThemeResourcePlugin[], activeId: string) => {
    localStorage.setItem(THEME_PLUGINS_KEY, JSON.stringify(plugins));
    localStorage.setItem(ACTIVE_THEME_KEY, activeId);
    setThemePlugins(plugins);
    setActiveThemeId(activeId);
  }, []);

  const recoverFailedMutation = useCallback(async (
    sequence: number,
    fallbackPlugins: ThemeResourcePlugin[],
    fallbackActiveId: string,
    error: unknown,
    message: string,
  ) => {
    console.warn(message, error);
    if (sequence !== mutationSequenceRef.current) return;
    try {
      const registry = await api.getAppearanceRegistry();
      if (sequence !== mutationSequenceRef.current) return;
      if (!registry || !Array.isArray(registry.plugins)) throw new Error('Appearance registry is unavailable');
      if (registry.revision < registryRevisionRef.current) return;
      applyRegistry(registry);
    } catch {
      if (sequence === mutationSequenceRef.current) restoreLocalSnapshot(fallbackPlugins, fallbackActiveId);
    }
  }, [applyRegistry, restoreLocalSnapshot]);

  useEffect(() => {
    let disposed = false;
    let started = false;
    let pending = false;
    let refreshAgain = false;
    let timer: number | undefined;
    const schedule = () => {
      window.clearTimeout(timer);
      if (!disposed && !document.hidden) timer = window.setTimeout(() => void refresh(), 30_000);
    };
    const refresh = async () => {
      if (disposed || document.hidden || !started) return;
      if (pending) { refreshAgain = true; return; }
      pending = true;
      window.clearTimeout(timer);
      try {
        const registry = await api.getAppearanceRegistry();
        if (!disposed && registry?.revision > registryRevisionRef.current) applyRegistry(registry);
      } catch { /* Events remain authoritative; the next visible refresh reconciles missed delivery. */ }
      finally { finishRefresh(); }
    };
    const finishRefresh = () => {
      pending = false;
      if (refreshAgain) { refreshAgain = false; void refresh(); }
      else schedule();
    };
    const hydrate = async () => {
      if (started || disposed) return;
      started = true;
      pending = true;
      hydrationRef.current ??= api.hydrateAppearanceRegistry(
        readThemePlugins(), localStorage.getItem(ACTIVE_THEME_KEY) ?? getInitialTheme(),
      );
      try {
        const registry = await hydrationRef.current;
        if (!disposed) applyRegistry(registry);
      } catch { hydrationRef.current = null; }
      finally { finishRefresh(); }
    };
    // Subscribe before hydration so another window or the agent cannot mutate
    // the registry between the initial read and listener registration.
    const connection = connectEventSubscriptions([
      (isActive) => listen<api.AppearanceRegistry>('appearance://changed', ({ payload }) => {
        if (!isActive() || !(payload?.revision > registryRevisionRef.current)) return;
        // Agent tool artifacts are compacted for display. Their event contains
        // only a revision; read the complete registry through its authority.
        if (Array.isArray(payload.plugins)) applyRegistry(payload);
        else void refresh();
      }),
    ], (state) => { if (state.status !== 'connecting') void hydrate(); });
    const onVisibility = () => {
      if (document.hidden) window.clearTimeout(timer);
      else void refresh();
    };
    document.addEventListener('visibilitychange', onVisibility);
    return () => {
      disposed = true;
      window.clearTimeout(timer);
      connection.stop();
      document.removeEventListener('visibilitychange', onVisibility);
    };
  }, [applyRegistry]);

  const setTheme = (newTheme: string) => {
    if (isThemeId(newTheme) || customThemes.some((profile) => profile.id === newTheme)) {
      const sequence = ++mutationSequenceRef.current;
      const previousActiveId = activeThemeId;
      setActiveThemeId(newTheme);
      void api.activateAppearance(newTheme)
        .then((registry) => { if (sequence === mutationSequenceRef.current) applyRegistry(registry); })
        .catch((error) => void recoverFailedMutation(
          sequence,
          themePlugins,
          previousActiveId,
          error,
          'Unable to persist active appearance',
        ));
    }
  };

  const installThemePlugin = (plugin: ThemeResourcePlugin) => {
    const normalized = normalizeThemeResourcePlugin(plugin);
    const sequence = ++mutationSequenceRef.current;
    const previousPlugins = themePlugins;
    const previousActiveId = activeThemeId;
    const next = [...themePlugins.filter((item) => item.id !== normalized.id), normalized];
    localStorage.setItem(THEME_PLUGINS_KEY, JSON.stringify(next));
    setThemePlugins(next);
    setActiveThemeId(normalized.id);
    void api.applyAppearancePlugin(normalized)
      .then((registry) => {
        if (sequence !== mutationSequenceRef.current) return;
        applyRegistry(registry);
        void collectUnusedThemeAssets(registry.plugins.map(themeResourcePluginToCustomTheme));
      })
      .catch((error) => void recoverFailedMutation(
        sequence,
        previousPlugins,
        previousActiveId,
        error,
        'Unable to persist appearance plugin',
      ));
  };

  const uninstallThemePlugin = (id: string, additionallyRetainedAssetIds: string[] = []) => {
    const sequence = ++mutationSequenceRef.current;
    const previousPlugins = themePlugins;
    const previousActiveId = activeThemeId;
    const next = themePlugins.filter((plugin) => plugin.id !== id);
    localStorage.setItem(THEME_PLUGINS_KEY, JSON.stringify(next));
    setThemePlugins(next);
    if (activeThemeId === id) setActiveThemeId('dark');
    void api.removeAppearance(id)
      .then((registry) => {
        if (sequence !== mutationSequenceRef.current) return;
        applyRegistry(registry);
        void collectUnusedThemeAssets(
          registry.plugins.map(themeResourcePluginToCustomTheme),
          additionallyRetainedAssetIds,
        );
      })
      .catch((error) => void recoverFailedMutation(
        sequence,
        previousPlugins,
        previousActiveId,
        error,
        'Unable to remove appearance plugin from the durable registry',
      ));
  };

  const rollbackTheme = () => {
    void api.rollbackAppearance().then(applyRegistry).catch((error) => {
      console.warn('Unable to roll back appearance', error);
    });
  };

  return (
    <ThemeContext.Provider value={{ theme, activeThemeId, customThemes, themePlugins, content, setTheme, installThemePlugin, uninstallThemePlugin, rollbackTheme }}>
      {children}
    </ThemeContext.Provider>
  );
}

async function collectUnusedThemeAssets(
  themes: CustomThemeDefinition[],
  additionallyRetainedAssetIds: string[] = [],
): Promise<void> {
  const retained = [
    ...themes.flatMap((theme) => theme.background.assetId ? [theme.background.assetId] : []),
    ...additionallyRetainedAssetIds,
  ];
  try {
    await api.garbageCollectThemeAssets(retained);
  } catch (error) {
    console.warn('Unable to garbage collect managed theme assets', error);
  }
}

const THEME_PLUGINS_KEY = 'nexa-theme-resource-plugins-v2';
const LEGACY_THEME_PLUGINS_KEY = 'nexa-theme-resource-plugins-v1';
const LEGACY_CUSTOM_THEMES_KEY = 'nexa-custom-themes-v1';
const ACTIVE_THEME_KEY = 'nexa-active-theme-v1';

function readThemePlugins(): ThemeResourcePlugin[] {
  try {
    const storedPlugins = localStorage.getItem(THEME_PLUGINS_KEY)
      ?? localStorage.getItem(LEGACY_THEME_PLUGINS_KEY);
    const value = JSON.parse(storedPlugins ?? localStorage.getItem(LEGACY_CUSTOM_THEMES_KEY) ?? '[]') as unknown;
    if (!Array.isArray(value)) return [];
    const plugins = value.flatMap((item) => {
      try {
        return [storedPlugins
          ? normalizeThemeResourcePlugin(item)
          : themeToResourcePlugin(item as CustomThemeDefinition)];
      } catch {
        return [];
      }
    });
    if (plugins.length > 0) localStorage.setItem(THEME_PLUGINS_KEY, JSON.stringify(plugins));
    return plugins;
  } catch {
    return [];
  }
}

export function useTheme() {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error('useTheme must be used within ThemeProvider');
  return ctx;
}
