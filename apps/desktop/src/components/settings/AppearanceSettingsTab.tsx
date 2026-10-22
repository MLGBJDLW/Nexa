import { ArrowRight, RotateCcw, Save, Star } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation, type Locale } from '../../i18n';
import * as api from '../../lib/api';
import type { Source } from '../../types';
import type { AppConfig } from '../../types/conversation';
import type { Project } from '../../types/project';
import { useTheme } from '../../lib/ThemeProvider';
import { isLightTheme } from '../../lib/theme';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';
import { CollapsiblePanel, Section } from './SettingsSection';
import { ToolApprovalControl, type ToolApprovalMode } from './ToolApprovalControl';
import { CompanionSettingsCard } from '../../features/companion/CompanionSettingsCard';
import { DisplaySettings } from './DisplaySettings';
import { KeyboardShortcutsSettings } from './KeyboardShortcutsSettings';
import { SettingsRow, settingsSelectClass } from './SettingsRow';
import { SettingsToggle } from './SettingsToggle';
import { NexaSelect } from '../ui/overlay';
import { ShellSettingsPanel } from './ShellSettingsPanel';


interface AppearanceSettingsTabProps {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  availableLocales: { code: Locale; name: string }[];
  appConfig: AppConfig | null;
  appConfigLoading: boolean;
  developerMode: boolean;
  onAppConfigChange: (config: AppConfig) => void;
  onAppConfigSave: (config?: AppConfig) => void;
  onDeveloperModeChange: (enabled: boolean) => void;
  onRerunWizard: () => void;
  onOpenThemeSettings: () => void;
}

function toggleId(ids: string[], id: string): string[] {
  return ids.includes(id) ? ids.filter((item) => item !== id) : [...ids, id];
}

function compactPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() || path;
}

export function AppearanceSettingsTab({
  locale,
  setLocale,
  availableLocales,
  appConfig,
  appConfigLoading,
  developerMode,
  onAppConfigChange,
  onAppConfigSave,
  onDeveloperModeChange,
  onRerunWizard,
  onOpenThemeSettings,
}: AppearanceSettingsTabProps) {
  const { t } = useTranslation();
  const { theme, activeThemeId, customThemes } = useTheme();
  const [dreamSources, setDreamSources] = useState<Source[]>([]);
  const [dreamProjects, setDreamProjects] = useState<Project[]>([]);
  const dreamingConfig = appConfig?.dreaming ?? {
    enabled: true,
    idle: false,
    afterScan: false,
    afterSuccessfulTurn: false,
    schedule: false,
    idleIntervalMinutes: 180,
    scheduleIntervalMinutes: 720,
    maxArtifactsPerRun: 24,
    maxRunsPerDay: 12,
    localOnly: true,
    sourceIds: [],
    projectIds: [],
  };

  useEffect(() => {
    let cancelled = false;
    void Promise.all([
      api.listSources().catch(() => [] as Source[]),
      api.listProjects().catch(() => [] as Project[]),
    ]).then(([sources, projects]) => {
      if (cancelled) return;
      setDreamSources(sources);
      setDreamProjects(projects.filter((project) => !project.archived));
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const updateDreamingConfig = (patch: Partial<typeof dreamingConfig>) => {
    if (!appConfig) return;
    onAppConfigChange({
      ...appConfig,
      dreaming: {
        ...dreamingConfig,
        ...patch,
      },
    });
  };
  const toggleDreamSource = (sourceId: string) => {
    updateDreamingConfig({
      sourceIds: toggleId(dreamingConfig.sourceIds ?? [], sourceId),
    });
  };
  const toggleDreamProject = (projectId: string) => {
    updateDreamingConfig({
      projectIds: toggleId(dreamingConfig.projectIds ?? [], projectId),
    });
  };
  const activeCustomTheme = customThemes.find((profile) => profile.id === activeThemeId);
  const activeThemeName = activeCustomTheme?.name
    ?? t(`settings.appearance.theme.${theme}`);
  const activeThemeMode = activeCustomTheme?.mode
    ?? (isLightTheme(theme) ? 'light' : 'dark');

  return (
    <Section icon={<Star size={20} />} title={t('settings.appearance')} delay={0.03}>
      <div className="space-y-3">
        {/* Theme section */}
        <div data-testid="theme-summary-card">
          <p className="mb-2 text-sm font-medium text-text-primary">{t('settings.appearance.theme')}</p>
          <p className="mb-3 text-xs text-text-tertiary">{t('settings.appearance.theme.description')}</p>
          <div className="flex flex-col gap-3 rounded-xl border border-border bg-surface-2 p-3 sm:flex-row sm:items-center">
            <div
              className="relative h-16 w-full shrink-0 overflow-hidden rounded-lg border border-border bg-surface-0 sm:w-24"
              data-testid="theme-summary-thumbnail"
              aria-hidden="true"
            >
              <div className="absolute inset-x-2 top-2 h-2 rounded-full bg-accent/80" />
              <div className="absolute bottom-2 left-2 top-6 w-5 rounded bg-surface-3" />
              <div className="absolute bottom-2 left-9 right-2 top-6 rounded border border-border bg-surface-1" />
            </div>
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm font-semibold text-text-primary">{activeThemeName}</p>
              <p className="mt-1 text-xs text-text-tertiary">
                {activeThemeMode === 'light' ? t('themeStudio.light') : t('themeStudio.dark')}
              </p>
            </div>
            <Button
              type="button"
              size="sm"
              variant="secondary"
              icon={<ArrowRight size={14} />}
              onClick={onOpenThemeSettings}
            >
              {t('themeStudio.openStudio')}
            </Button>
          </div>
        </div>

        {appConfig && <ShellSettingsPanel config={appConfig} saving={appConfigLoading} onChange={onAppConfigChange} onSave={() => onAppConfigSave()} />}

        <CollapsiblePanel title={t('settings.displayFonts')} description={t('settings.displayStreaming')} testId="display-preferences">
          <DisplaySettings />
        </CollapsiblePanel>

        <KeyboardShortcutsSettings />

        {/* Separator */}
        <div className="border-t border-border" />

        <CollapsiblePanel title={t('companion.title')} testId="companion-preferences">
          <CompanionSettingsCard appConfig={appConfig} loading={appConfigLoading} onChange={onAppConfigChange} onSave={(next) => onAppConfigSave(next)} />
        </CollapsiblePanel>

        <SettingsRow label={t('settings.appearance.language')} description={t('settings.appearance.language.description')} htmlFor="settings-language">
          <select id="settings-language" value={locale} onChange={event => setLocale(event.target.value as Locale)} className={settingsSelectClass}>
            {availableLocales.map(language => <option key={language.code} value={language.code}>{language.name}</option>)}
          </select>
        </SettingsRow>
        <SettingsRow label={t('settings.windowCloseBehavior')} description={t('settings.windowCloseBehaviorDesc')} htmlFor="settings-close-behavior">
          <select id="settings-close-behavior" disabled={!appConfig || appConfigLoading} value={appConfig?.windowCloseBehavior ?? 'exit'} className={settingsSelectClass}
            onChange={event => {
              if (!appConfig) return;
              const next = { ...appConfig, windowCloseBehavior: event.target.value as 'exit' | 'minimize_to_tray' };
              onAppConfigChange(next);
              onAppConfigSave(next);
            }}>
            <option value="exit">{t('settings.windowCloseExit')}</option>
            <option value="minimize_to_tray">{t('settings.windowCloseTray')}</option>
          </select>
        </SettingsRow>

        {/* App update */}

        <div className="border-t border-border pt-3">
          <label className="flex cursor-pointer items-start gap-3">
            <input
              type="checkbox"
              checked={developerMode}
              onChange={(event) => onDeveloperModeChange(event.target.checked)}
              className="mt-0.5 h-4 w-4 rounded border-border text-accent focus:ring-accent/30"
            />
            <span>
              <span className="block text-sm font-medium text-text-primary">
                {t('settings.developerMode')}
              </span>
              <span className="mt-1 block text-xs leading-relaxed text-text-tertiary">
                {t('settings.developerMode.description')}
              </span>
            </span>
          </label>
        </div>

        {/* Re-run setup wizard */}
        <div className="border-t border-border pt-3">
          <p className="mb-2 text-sm font-medium text-text-primary">{t('wizard.rerunLabel')}</p>
          <p className="mb-3 text-xs text-text-tertiary">{t('wizard.rerunDescription')}</p>
          <Button
            variant="secondary"
            size="sm"
            icon={<RotateCcw size={14} />}
            onClick={onRerunWizard}
          >
            {t('wizard.rerunButton')}
          </Button>
        </div>

        {appConfig && (
          <div className="space-y-2" data-testid="context-management-settings">
            <SettingsRow label={t('settings.contextManagementMode')} description={appConfig.contextManagementMode === 'history' ? t('settings.contextHistoryHelp') : t('settings.contextSummaryHelp')} htmlFor="context-management-mode">
            <select id="context-management-mode"
              className="w-full rounded-lg border border-border bg-surface-2 px-3 py-2 text-sm text-text-primary"
              value={appConfig.contextManagementMode ?? 'summary'}
              onChange={event => onAppConfigChange({ ...appConfig, contextManagementMode: event.target.value as 'summary' | 'history' })}>
              <option value="summary">{t('settings.contextModeSummary')}</option>
              <option value="history">{t('settings.contextModeHistory')}</option>
            </select>
            </SettingsRow>
            <p className="text-xs leading-5 text-text-tertiary">{t('settings.contextModeScope')}</p>
            <div data-settings-actions className="flex justify-end">
              <Button variant="primary" size="sm" icon={<Save size={14} />} loading={appConfigLoading} onClick={() => onAppConfigSave()}>
                {t('common.save')}
              </Button>
            </div>
          </div>
        )}

        {appConfig && <div className="space-y-3" data-testid="appearance-runtime-settings">
          <CollapsiblePanel title={t('nav.search')} testId="search-preferences">
            <SettingsRow label={t('settings.searchLimit')} description={t('settings.searchLimitDesc')} htmlFor="settings-search-limit">
              <Input id="settings-search-limit" type="number" value={appConfig.defaultSearchLimit} min={1} max={100} onChange={event => onAppConfigChange({ ...appConfig, defaultSearchLimit: Math.max(1, Math.min(100, parseInt(event.target.value) || 20)) })} />
            </SettingsRow>
            <SettingsRow label={t('settings.searchSimilarity')} description={t('settings.searchSimilarityDesc')} htmlFor="settings-search-similarity">
              <Input id="settings-search-similarity" type="number" value={appConfig.minSearchSimilarity} min={0} max={1} step={0.05} onChange={event => {
                const parsed = Number.parseFloat(event.target.value);
                onAppConfigChange({ ...appConfig, minSearchSimilarity: Math.max(0, Math.min(1, Number.isFinite(parsed) ? parsed : 0.2)) });
              }} />
            </SettingsRow>
          </CollapsiblePanel>
          <CollapsiblePanel title={t('settings.fileSizeLimits')} testId="file-size-preferences">
            {([
              ['maxTextFileSize', 'settings.maxTextFileSize', 1024 * 1024, 100, 1024],
              ['maxVideoFileSize', 'settings.maxVideoFileSize', 1024 * 1024 * 1024, 2, 10],
              ['maxAudioFileSize', 'settings.maxAudioFileSize', 1024 * 1024, 500, 2048],
            ] as const).map(([key, label, unit, fallback, max]) => <SettingsRow key={key} label={t(label)} description={key === 'maxTextFileSize' ? t('settings.maxTextFileSizeDesc') : undefined} htmlFor={`settings-${key}`}>
              <Input id={`settings-${key}`} type="number" value={Math.round(appConfig[key] / unit)} min={1} max={max} onChange={event => onAppConfigChange({ ...appConfig, [key]: Math.max(1, parseInt(event.target.value) || fallback) * unit })} />
            </SettingsRow>)}
          </CollapsiblePanel>
          <CollapsiblePanel title={t('settings.agentBehaviorGroup')} testId="agent-behavior-preferences">
            {([
              ['dynamicToolVisibility', 'settings.dynamicTools', 'settings.dynamicToolsDesc', false],
              ['traceEnabled', 'settings.traceEnabled', 'settings.traceEnabledDesc', true],
              ['autoMemoryExtraction', 'settings.autoMemoryExtraction', 'settings.autoMemoryExtractionDesc', true],
              ['autoSkillLearning', 'settings.autoSkillLearning', 'settings.autoSkillLearningDesc', true],
            ] as const).map(([key, label, description, fallback]) => <SettingsRow key={key} label={t(label)} description={t(description)} htmlFor={`settings-${key}`}>
              <SettingsToggle id={`settings-${key}`} label={t(label)} checked={appConfig[key] ?? fallback} onChange={checked => onAppConfigChange({ ...appConfig, [key]: checked })} />
            </SettingsRow>)}
          </CollapsiblePanel>
          <CollapsiblePanel title={t('settings.dreamingEnabled')} testId="dreaming-preferences">
            <SettingsRow label={t('settings.dreamingEnabled')} description={t('settings.dreamingEnabledDesc')} htmlFor="settings-dreaming-enabled">
              <SettingsToggle id="settings-dreaming-enabled" label={t('settings.dreamingEnabled')} checked={dreamingConfig.enabled} onChange={enabled => updateDreamingConfig({ enabled })} />
            </SettingsRow>
            <div className="space-y-3">
              <CollapsiblePanel title={t('settings.dreamingTriggers')}>
                {([
                  ['idle', 'settings.dreamingIdle', 'settings.dreamingIdleDesc'],
                  ['afterScan', 'settings.dreamingAfterScan', 'settings.dreamingAfterScanDesc'],
                  ['afterSuccessfulTurn', 'settings.dreamingAfterTurn', 'settings.dreamingAfterTurnDesc'],
                  ['schedule', 'settings.dreamingSchedule', 'settings.dreamingScheduleDesc'],
                ] as const).map(([key, label, description]) => <SettingsRow key={key} label={t(label)} description={t(description)} htmlFor={`settings-dreaming-${key}`}>
                  <SettingsToggle id={`settings-dreaming-${key}`} label={t(label)} checked={dreamingConfig[key]} disabled={!dreamingConfig.enabled} onChange={checked => updateDreamingConfig({ [key]: checked })} />
                </SettingsRow>)}
                <SettingsRow label={t('settings.dreamingIdleInterval')} htmlFor="settings-dreaming-idle-interval">
                  <Input id="settings-dreaming-idle-interval" type="number" value={dreamingConfig.idleIntervalMinutes} min={15} max={1440} disabled={!dreamingConfig.enabled || !dreamingConfig.idle} onChange={event => updateDreamingConfig({ idleIntervalMinutes: Math.max(15, Math.min(1440, parseInt(event.target.value) || 180)) })} />
                </SettingsRow>
                <SettingsRow label={t('settings.dreamingScheduleInterval')} htmlFor="settings-dreaming-schedule-interval">
                  <Input id="settings-dreaming-schedule-interval" type="number" value={dreamingConfig.scheduleIntervalMinutes} min={30} max={10080} disabled={!dreamingConfig.enabled || !dreamingConfig.schedule} onChange={event => updateDreamingConfig({ scheduleIntervalMinutes: Math.max(30, Math.min(10080, parseInt(event.target.value) || 720)) })} />
                </SettingsRow>
              </CollapsiblePanel>
              <CollapsiblePanel title={t('settings.dreamingScope')}>
                <SettingsRow label={t('settings.dreamingMaxArtifacts')} description={t('settings.dreamingMaxArtifactsDesc')} htmlFor="settings-dreaming-max-artifacts">
                  <Input id="settings-dreaming-max-artifacts" type="number" value={dreamingConfig.maxArtifactsPerRun} min={1} max={100} disabled={!dreamingConfig.enabled} onChange={event => updateDreamingConfig({ maxArtifactsPerRun: Math.max(1, Math.min(100, parseInt(event.target.value) || 24)) })} />
                </SettingsRow>
                <SettingsRow label={t('settings.dreamingMaxRuns')} description={t('settings.dreamingMaxRunsDesc')} htmlFor="settings-dreaming-max-runs">
                  <Input id="settings-dreaming-max-runs" type="number" value={dreamingConfig.maxRunsPerDay} min={0} max={96} disabled={!dreamingConfig.enabled} onChange={event => updateDreamingConfig({ maxRunsPerDay: Math.max(0, Math.min(96, parseInt(event.target.value) || 0)) })} />
                </SettingsRow>
                <SettingsRow label={t('settings.dreamingLocalOnly')} description={t('settings.dreamingLocalOnlyDesc')} htmlFor="settings-dreaming-local">
                  <SettingsToggle id="settings-dreaming-local" label={t('settings.dreamingLocalOnly')} checked={dreamingConfig.localOnly} disabled={!dreamingConfig.enabled} onChange={localOnly => updateDreamingConfig({ localOnly })} />
                </SettingsRow>
                  <div className="space-y-3">
                    <div className="rounded-md border border-border bg-surface-0 p-3">
                      <div className="flex items-center justify-between gap-2">
                        <div>
                          <div className="text-sm font-medium text-text-primary">{t('settings.dreamingSources')}</div>
                          <p className="mt-1 text-xs text-text-tertiary">{t('settings.dreamingSourcesDesc')}</p>
                        </div>
                        {(dreamingConfig.sourceIds?.length ?? 0) > 0 && (
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => updateDreamingConfig({ sourceIds: [] })}
                            disabled={!dreamingConfig.enabled}
                          >
                            {t('settings.dreamingAll')}
                          </Button>
                        )}
                      </div>
                      <div className="mt-3 max-h-40 space-y-2 overflow-y-auto">
                        {dreamSources.length === 0 ? (
                          <p className="text-xs text-text-tertiary">{t('settings.dreamingNoSources')}</p>
                        ) : dreamSources.map((source) => (
                          <label key={source.id} className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 hover:bg-surface-2">
                            <input
                              type="checkbox"
                              checked={(dreamingConfig.sourceIds ?? []).includes(source.id)}
                              disabled={!dreamingConfig.enabled}
                              onChange={() => toggleDreamSource(source.id)}
                              className="rounded border-border"
                            />
                            <span className="min-w-0 flex-1 truncate text-sm text-text-primary" title={source.rootPath}>
                              {compactPath(source.rootPath)}
                            </span>
                          </label>
                        ))}
                      </div>
                    </div>

                    <div className="rounded-md border border-border bg-surface-0 p-3">
                      <div className="flex items-center justify-between gap-2">
                        <div>
                          <div className="text-sm font-medium text-text-primary">{t('settings.dreamingProjects')}</div>
                          <p className="mt-1 text-xs text-text-tertiary">{t('settings.dreamingProjectsDesc')}</p>
                        </div>
                        {(dreamingConfig.projectIds?.length ?? 0) > 0 && (
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => updateDreamingConfig({ projectIds: [] })}
                            disabled={!dreamingConfig.enabled}
                          >
                            {t('settings.dreamingAll')}
                          </Button>
                        )}
                      </div>
                      <div className="mt-3 max-h-40 space-y-2 overflow-y-auto">
                        {dreamProjects.length === 0 ? (
                          <p className="text-xs text-text-tertiary">{t('settings.dreamingNoProjects')}</p>
                        ) : dreamProjects.map((project) => (
                          <label key={project.id} className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 hover:bg-surface-2">
                            <input
                              type="checkbox"
                              checked={(dreamingConfig.projectIds ?? []).includes(project.id)}
                              disabled={!dreamingConfig.enabled}
                              onChange={() => toggleDreamProject(project.id)}
                              className="rounded border-border"
                            />
                            <span className="min-w-0 flex-1 truncate text-sm text-text-primary" title={project.name}>
                              {project.name}
                            </span>
                          </label>
                        ))}
                      </div>
                    </div>
                  </div>
              </CollapsiblePanel>
            </div>
          </CollapsiblePanel>
          <CollapsiblePanel title={t('settings.packageHost.permissions')} testId="permissions-preferences">
            <SettingsRow label={t('settings.confirmDestructive')} description={t('settings.confirmDestructiveDesc')} htmlFor="settings-confirm-destructive">
              <SettingsToggle id="settings-confirm-destructive" label={t('settings.confirmDestructive')} checked={appConfig.confirmDestructive ?? false} onChange={confirmDestructive => onAppConfigChange({ ...appConfig, confirmDestructive })} />
            </SettingsRow>
            <SettingsRow label={t('settings.shellAccessMode')} description={t('settings.shellAccessModeDesc')} htmlFor="settings-shell-access">
              <NexaSelect id="settings-shell-access" value={appConfig.shellAccessMode ?? 'restricted'} onChange={event => onAppConfigChange({ ...appConfig, shellAccessMode: event.target.value as 'restricted' | 'confirm_all' | 'open' })} className={settingsSelectClass}>
                <option value="restricted">{t('settings.shellAccessRestricted')}</option>
                <option value="confirm_all">{t('settings.shellAccessConfirmAll')}</option>
                <option value="open">{t('settings.shellAccessOpen')}</option>
              </NexaSelect>
            </SettingsRow>
            <ToolApprovalControl mode={appConfig.toolApprovalMode ?? 'ask'} onChange={(toolApprovalMode: ToolApprovalMode) => onAppConfigChange({ ...appConfig, toolApprovalMode })} />
          </CollapsiblePanel>
          <div data-settings-actions className="flex justify-end"><Button variant="primary" size="sm" icon={<Save size={14} />} loading={appConfigLoading} onClick={() => onAppConfigSave()}>{t('common.save')}</Button></div>
        </div>}
      </div>
    </Section>
  );
}
