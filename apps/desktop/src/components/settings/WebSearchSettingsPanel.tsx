import { ChevronDown, GripVertical, RefreshCw, SearchCheck } from 'lucide-react';
import { DndContext, KeyboardSensor, PointerSensor, closestCenter, useSensor, useSensors, type DragEndEvent } from '@dnd-kit/core';
import { SortableContext, arrayMove, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from '@dnd-kit/sortable';
import { restrictToParentElement, restrictToVerticalAxis } from '@dnd-kit/modifiers';
import { CSS } from '@dnd-kit/utilities';
import { NexaSelect } from '../ui/overlay';
import { useEffect, useMemo, useState } from 'react';
import { useTranslation, type TranslationKey } from '../../i18n';
import * as api from '../../lib/api';
import type {
  AppConfig,
  ProviderNativeSearchEngine,
  WebSearchCustomProviderConfig,
  WebSearchCustomProviderPreset,
  WebSearchConfig,
  WebSearchExecutionMode,
  WebSearchProviderHealth,
  WebSearchProviderMode,
  WebSearchProviderProfile,
  WebSearchProviderStatus,
  WebSearchReranker,
} from '../../types/conversation';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';
import { Section } from './SettingsSection';
import { SettingsRow, settingsSelectClass } from './SettingsRow';
import { PROVIDER_PRESETS } from '../../lib/providerPresets';
import type { NativeWebSearchCapability } from '../../lib/modelCatalog';

interface WebSearchSettingsPanelProps {
  appConfig: AppConfig;
  loading: boolean;
  onChange: (config: AppConfig) => void;
  onMarkDirty: () => void;
  onSave: () => void;
}

const DEFAULT_WEB_SEARCH_CONFIG: WebSearchConfig = {
  executionMode: 'auto',
  providerNativeEngine: 'auto',
  providerProfile: 'default',
  reranker: 'auto',
  providerMode: 'built_in_first',
  customProviders: [
    { id: 'deepseek', preset: 'deepseek', name: 'DeepSeek Search & Fetch', enabled: false, apiKey: '', baseUrl: 'https://api.deepseek.com/v1/tools/search', priority: 15 },
    {
      id: 'brave',
      preset: 'brave',
      name: 'Brave Search API',
      enabled: false,
      apiKey: '',
      baseUrl: 'https://api.search.brave.com/res/v1/web/search',
      priority: 10,
    },
    {
      id: 'tavily',
      preset: 'tavily',
      name: 'Tavily Search',
      enabled: false,
      apiKey: '',
      baseUrl: 'https://api.tavily.com/search',
      priority: 20,
    },
    {
      id: 'anysearch',
      preset: 'anysearch',
      name: 'AnySearch',
      enabled: false,
      apiKey: '',
      baseUrl: 'https://api.anysearch.com/v1/search',
      priority: 25,
    },
    {
      id: 'serpapi_google',
      preset: 'serpapi_google',
      name: 'SerpAPI Google',
      enabled: false,
      apiKey: '',
      baseUrl: 'https://serpapi.com/search.json',
      priority: 30,
    },
    {
      id: 'searxng',
      preset: 'searxng',
      name: 'SearXNG',
      enabled: false,
      apiKey: '',
      baseUrl: '',
      priority: 40,
    },
  ],
};

const PROFILE_OPTIONS: WebSearchProviderProfile[] = [
  'default',
  'free',
  'free_verified',
  'max_evidence',
];

const RERANKER_OPTIONS: WebSearchReranker[] = [
  'auto',
  'none',
  'docs_first',
  'research',
  'news_balanced',
];

const PROVIDER_MODE_OPTIONS: WebSearchProviderMode[] = [
  'built_in_first',
  'custom_first',
  'custom_only',
];

const EXECUTION_MODE_OPTIONS: WebSearchExecutionMode[] = [
  'auto',
  'providerNative',
  'nexaRouter',
  'hybrid',
];

const PROVIDER_NATIVE_ENGINE_OPTIONS: ProviderNativeSearchEngine[] = [
  'auto',
  'native',
  'exa',
  'firecrawl',
  'parallel',
  'perplexity',
];

const PROVIDER_NATIVE_ENGINE_LABELS: Record<ProviderNativeSearchEngine, string> = {
  auto: 'Auto',
  native: 'Native',
  exa: 'Exa',
  firecrawl: 'Firecrawl',
  parallel: 'Parallel',
  perplexity: 'Perplexity',
};

type NativeSearchSupportEntry = {
  id: string;
  name: string;
  capability: NativeWebSearchCapability;
  modelCount: number;
  supportsGoogleGrounding: boolean;
  separatesClientTools: boolean;
};

const NATIVE_SEARCH_SUPPORT: NativeSearchSupportEntry[] = PROVIDER_PRESETS.flatMap((preset) => {
  const modelCapabilities = preset.models.flatMap((model) => {
    const capability = model.descriptor.capabilities.nativeWebSearch ?? preset.nativeWebSearch;
    return capability ? [capability] : [];
  });
  const capability = modelCapabilities[0] ?? preset.nativeWebSearch;
  if (!capability) return [];
  return [{
    id: preset.id,
    name: preset.name,
    capability,
    modelCount: modelCapabilities.length,
    supportsGoogleGrounding: capability.dialect === 'geminiGoogleSearch' || modelCapabilities.some(model => model.dialect === 'geminiGoogleSearch'),
    separatesClientTools: modelCapabilities.some(model => !model.canMixClientTools),
  }];
});

const PROFILE_LABEL_KEYS: Record<WebSearchProviderProfile, TranslationKey> = {
  default: 'settings.webSearchProfile.default',
  free: 'settings.webSearchProfile.free',
  free_verified: 'settings.webSearchProfile.free_verified',
  max_evidence: 'settings.webSearchProfile.max_evidence',
};

const PROFILE_DESC_KEYS: Record<WebSearchProviderProfile, TranslationKey> = {
  default: 'settings.webSearchProfile.default.desc',
  free: 'settings.webSearchProfile.free.desc',
  free_verified: 'settings.webSearchProfile.free_verified.desc',
  max_evidence: 'settings.webSearchProfile.max_evidence.desc',
};

const RERANKER_LABEL_KEYS: Record<WebSearchReranker, TranslationKey> = {
  auto: 'settings.webSearchReranker.auto',
  none: 'settings.webSearchReranker.none',
  docs_first: 'settings.webSearchReranker.docs_first',
  research: 'settings.webSearchReranker.research',
  news_balanced: 'settings.webSearchReranker.news_balanced',
};

const RERANKER_DESC_KEYS: Record<WebSearchReranker, TranslationKey> = {
  auto: 'settings.webSearchReranker.auto.desc',
  none: 'settings.webSearchReranker.none.desc',
  docs_first: 'settings.webSearchReranker.docs_first.desc',
  research: 'settings.webSearchReranker.research.desc',
  news_balanced: 'settings.webSearchReranker.news_balanced.desc',
};

const HEALTH_LABEL_KEYS: Record<WebSearchProviderHealth, TranslationKey> = {
  healthy: 'settings.webSearchHealth.healthy',
  degraded: 'settings.webSearchHealth.degraded',
  blocked: 'settings.webSearchHealth.blocked',
  disabled: 'settings.webSearchHealth.disabled',
};

const PROVIDER_MODE_LABEL_KEYS: Record<WebSearchProviderMode, TranslationKey> = {
  built_in_first: 'settings.webSearchProviderMode.built_in_first',
  custom_first: 'settings.webSearchProviderMode.custom_first',
  custom_only: 'settings.webSearchProviderMode.custom_only',
};

const PROVIDER_MODE_DESC_KEYS: Record<WebSearchProviderMode, TranslationKey> = {
  built_in_first: 'settings.webSearchProviderMode.built_in_first.desc',
  custom_first: 'settings.webSearchProviderMode.custom_first.desc',
  custom_only: 'settings.webSearchProviderMode.custom_only.desc',
};

const EXECUTION_MODE_LABEL_KEYS: Record<WebSearchExecutionMode, TranslationKey> = {
  auto: 'settings.webSearchExecutionMode.auto',
  providerNative: 'settings.webSearchExecutionMode.providerNative',
  nexaRouter: 'settings.webSearchExecutionMode.nexaRouter',
  hybrid: 'settings.webSearchExecutionMode.hybrid',
};

const EXECUTION_MODE_DESC_KEYS: Record<WebSearchExecutionMode, TranslationKey> = {
  auto: 'settings.webSearchExecutionMode.auto.desc',
  providerNative: 'settings.webSearchExecutionMode.providerNative.desc',
  nexaRouter: 'settings.webSearchExecutionMode.nexaRouter.desc',
  hybrid: 'settings.webSearchExecutionMode.hybrid.desc',
};

const CUSTOM_PROVIDER_LABEL_KEYS: Record<WebSearchCustomProviderPreset, TranslationKey> = {
  deepseek: 'settings.webSearchCustomProvider.deepseek',
  brave: 'settings.webSearchCustomProvider.brave',
  tavily: 'settings.webSearchCustomProvider.tavily',
  anysearch: 'settings.webSearchCustomProvider.anysearch',
  serpapi_google: 'settings.webSearchCustomProvider.serpapi_google',
  searxng: 'settings.webSearchCustomProvider.searxng',
};

const CUSTOM_PROVIDER_DESC_KEYS: Record<WebSearchCustomProviderPreset, TranslationKey> = {
  deepseek: 'settings.webSearchCustomProvider.deepseek.desc',
  brave: 'settings.webSearchCustomProvider.brave.desc',
  tavily: 'settings.webSearchCustomProvider.tavily.desc',
  anysearch: 'settings.webSearchCustomProvider.anysearch.desc',
  serpapi_google: 'settings.webSearchCustomProvider.serpapi_google.desc',
  searxng: 'settings.webSearchCustomProvider.searxng.desc',
};

function mergeCustomProviders(
  providers: WebSearchCustomProviderConfig[] | undefined,
): WebSearchCustomProviderConfig[] {
  const byPreset = new Map<WebSearchCustomProviderPreset, WebSearchCustomProviderConfig>();
  for (const provider of providers ?? []) {
    byPreset.set(provider.preset, provider);
  }
  return DEFAULT_WEB_SEARCH_CONFIG.customProviders.map((fallback) => ({
    ...fallback,
    ...(byPreset.get(fallback.preset) ?? {}),
  }));
}

function webSearchConfig(appConfig: AppConfig): WebSearchConfig {
  const incoming = appConfig.webSearch ?? DEFAULT_WEB_SEARCH_CONFIG;
  return {
    ...DEFAULT_WEB_SEARCH_CONFIG,
    ...incoming,
    customProviders: mergeCustomProviders(incoming.customProviders),
  };
}

function healthClass(health: WebSearchProviderHealth): string {
  switch (health) {
    case 'healthy':
      return 'bg-success/10 text-success';
    case 'degraded':
      return 'bg-warning/10 text-warning';
    case 'blocked':
    case 'disabled':
      return 'bg-danger/10 text-danger';
    default:
      return 'bg-surface-3 text-text-tertiary';
  }
}

function isCustomProviderConfigured(provider: WebSearchCustomProviderConfig): boolean {
  if (!provider.enabled) return false;
  if (provider.preset === 'searxng') return Boolean(provider.baseUrl?.trim());
  if (provider.preset === 'anysearch') return true;
  return Boolean(provider.apiKey.trim());
}

const SELECT_CLASS = settingsSelectClass;

function SearchProviderRow({ provider, status, expanded, onExpand, onChange }: {
  provider: WebSearchCustomProviderConfig;
  status?: WebSearchProviderStatus;
  expanded: boolean;
  onExpand: () => void;
  onChange: (patch: Partial<WebSearchCustomProviderConfig>) => void;
}) {
  const { t } = useTranslation();
  const { attributes, listeners, setNodeRef, setActivatorNodeRef, transform, transition, isDragging } = useSortable({ id: provider.id });
  const label = t(CUSTOM_PROVIDER_LABEL_KEYS[provider.preset]);
  const health = !provider.enabled ? 'disabled' : status?.health;
  return <div
    ref={setNodeRef}
    style={{ transform: CSS.Transform.toString(transform), transition, zIndex: isDragging ? 10 : undefined }}
    className={`relative min-w-0 bg-surface-1 ${isDragging ? 'rounded-lg shadow-lg ring-1 ring-accent/40' : ''}`}
    data-testid={`web-search-source-${provider.preset}`}
  >
    <div className="flex min-h-12 min-w-0 items-center gap-2 px-2 py-1.5">
      <button
        ref={setActivatorNodeRef}
        type="button"
        {...attributes}
        {...listeners}
        aria-label={t('settings.webSearchReorder', { name: label })}
        className="flex h-8 w-7 shrink-0 touch-none cursor-grab items-center justify-center rounded text-text-tertiary hover:bg-surface-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent active:cursor-grabbing"
      ><GripVertical size={15} /></button>
      <button type="button" onClick={onExpand} aria-expanded={expanded} aria-controls={`web-search-config-${provider.preset}`} className="min-w-0 flex-1 rounded py-1 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent">
        <span className="block truncate text-sm font-medium text-text-primary">{label}</span>
        <span className="block truncate text-[11px] text-text-tertiary">
          {provider.enabled && !isCustomProviderConfigured(provider)
            ? t('settings.webSearchProviderNeedsConfig')
            : health ? t(HEALTH_LABEL_KEYS[health]) : t('settings.webSearchCredentials')}
        </span>
      </button>
      <NexaSelect aria-label={`${label}: ${t('settings.webSearchSourceState')}`} value={provider.enabled ? 'enabled' : 'disabled'} onChange={event => onChange({ enabled: event.target.value === 'enabled' })} className={`${SELECT_CLASS} !w-28`}>
        <option value="enabled">{t('settings.packageHost.state.enabled')}</option>
        <option value="disabled">{t('settings.packageHost.state.disabled')}</option>
      </NexaSelect>
      <button type="button" onClick={onExpand} aria-expanded={expanded} aria-controls={`web-search-config-${provider.preset}`} aria-label={`${expanded ? t('common.collapse') : t('common.expand')}: ${label}`} className="flex h-8 w-7 shrink-0 items-center justify-center rounded text-text-tertiary hover:bg-surface-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent">
        <ChevronDown size={15} className={expanded ? 'rotate-180' : ''} />
      </button>
    </div>
    {expanded && <div id={`web-search-config-${provider.preset}`} className="space-y-3 px-4 pb-4 pt-1 sm:pl-11">
      <p className="text-xs leading-relaxed text-text-tertiary">{t(CUSTOM_PROVIDER_DESC_KEYS[provider.preset])}</p>
      {provider.preset !== 'searxng' && <label className="block space-y-1.5 text-xs text-text-secondary">
        <span>{t('settings.webSearchProviderApiKey')}</span>
        <Input type="password" autoComplete="off" value={provider.apiKey} placeholder={t('settings.webSearchProviderApiKeyPlaceholder')} onChange={event => onChange({ apiKey: event.target.value })} />
      </label>}
      <label className="block space-y-1.5 text-xs text-text-secondary">
        <span>{t('settings.webSearchProviderBaseUrl')}</span>
        <Input value={provider.baseUrl ?? ''} placeholder={provider.preset === 'searxng' ? 'https://searx.example.org' : undefined} onChange={event => onChange({ baseUrl: event.target.value })} />
      </label>
      {status?.lastErrorCode && <p className="text-xs text-text-tertiary">{t('settings.webSearchLastError')}: {status.lastErrorCode}{status.nextRetrySeconds ? `, ${t('settings.webSearchRetryIn', { seconds: status.nextRetrySeconds })}` : ''}</p>}
    </div>}
  </div>;
}

export function WebSearchSettingsPanel({
  appConfig,
  loading,
  onChange,
  onMarkDirty,
  onSave,
}: WebSearchSettingsPanelProps) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<WebSearchProviderStatus[]>([]);
  const [refreshing, setRefreshing] = useState(false);
  const [expandedProvider, setExpandedProvider] = useState<string | null>(null);
  const config = useMemo(() => webSearchConfig(appConfig), [appConfig]);
  const orderedProviders = useMemo(() => [...config.customProviders].sort((left, right) => left.priority - right.priority), [config.customProviders]);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  const refreshStatus = async () => {
    setRefreshing(true);
    try {
      setStatus(await api.getWebSearchStatus(config));
    } catch {
      setStatus([]);
    } finally {
      setRefreshing(false);
    }
  };

  useEffect(() => {
    void refreshStatus();
  }, [config.providerProfile, config.providerMode, config.customProviders]);

  const updateConfig = (webSearch: WebSearchConfig) => {
    onChange({ ...appConfig, webSearch });
    onMarkDirty();
  };

  const updateCustomProvider = (
    preset: WebSearchCustomProviderPreset,
    patch: Partial<WebSearchCustomProviderConfig>,
  ) => {
    updateConfig({
      ...config,
      customProviders: config.customProviders.map((provider) =>
        provider.preset === preset ? { ...provider, ...patch } : provider,
      ),
    });
  };

  const statusById = new Map(status.map((provider) => [provider.id, provider]));
  const configuredProviderCount = config.customProviders.filter(isCustomProviderConfigured).length;
  const reorderProviders = ({ active, over }: DragEndEvent) => {
    if (!over || active.id === over.id) return;
    const from = orderedProviders.findIndex(provider => provider.id === active.id);
    const to = orderedProviders.findIndex(provider => provider.id === over.id);
    if (from < 0 || to < 0) return;
    updateConfig({
      ...config,
      customProviders: arrayMove(orderedProviders, from, to).map((provider, index) => ({ ...provider, priority: index + 1 })),
    });
  };

  return (
    <Section
      icon={<SearchCheck size={20} />}
      title={t('settings.webSearchTitle')}
      description={t('settings.webSearchDesc')}
      collapsible
      defaultOpen={false}
      summary={<span className="rounded-full border border-border/60 bg-surface-2 px-2 py-1 text-[11px] text-text-secondary">{configuredProviderCount}/{config.customProviders.length}</span>}
    >
      <div className="min-w-0 space-y-3" data-testid="web-search-settings">
        <SettingsRow label={t('settings.webSearchExecutionMode')} description={t(EXECUTION_MODE_DESC_KEYS[config.executionMode])}>
          <NexaSelect aria-label={t('settings.webSearchExecutionMode')} value={config.executionMode} onChange={event => updateConfig({ ...config, executionMode: event.target.value as WebSearchExecutionMode })} className={SELECT_CLASS}>
            {EXECUTION_MODE_OPTIONS.map(mode => <option key={mode} value={mode}>{t(EXECUTION_MODE_LABEL_KEYS[mode])}</option>)}
          </NexaSelect>
        </SettingsRow>

        <div className="space-y-2" data-testid="provider-native-search-support">
          <div>
            <p className="text-sm font-medium text-text-primary">{t('settings.webSearchNativeSupport')}</p>
            <p className="mt-1 text-xs leading-relaxed text-text-tertiary">{t('settings.webSearchNativeSupport.desc')}</p>
          </div>
          <div className="divide-y divide-border/50 overflow-hidden rounded-lg border border-border/60">
            {NATIVE_SEARCH_SUPPORT.map(({ id, name, capability, modelCount, supportsGoogleGrounding, separatesClientTools }) => (
              <div key={id} data-testid={`native-search-source-${id}`} className="flex min-w-0 items-center justify-between gap-3 px-3 py-2">
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium text-text-primary">{name}</p>
                  <p className="mt-0.5 text-[11px] text-text-tertiary">
                    {supportsGoogleGrounding ? t('settings.webSearchGrounding') : t('settings.webSearchExecutionMode.providerNative')}
                    {' · '}{t('settings.webSearchSupportedModels', { count: modelCount })}
                    {separatesClientTools && <span className="block">{t('settings.webSearchSeparateTools')}</span>}
                  </p>
                </div>
                {capability.dialect === 'openRouterServerTool' && <label className="w-36 max-w-[42%] shrink-0">
                  <span className="sr-only">{t('settings.webSearchProviderNativeEngine')}</span>
                  <NexaSelect aria-label={t('settings.webSearchProviderNativeEngine')} title={t('settings.webSearchProviderNativeEngine.desc')} value={config.providerNativeEngine} onChange={event => updateConfig({ ...config, providerNativeEngine: event.target.value as ProviderNativeSearchEngine })} className={SELECT_CLASS}>
                    {PROVIDER_NATIVE_ENGINE_OPTIONS.map(engine => <option key={engine} value={engine}>{PROVIDER_NATIVE_ENGINE_LABELS[engine]}</option>)}
                  </NexaSelect>
                </label>}
              </div>
            ))}
          </div>
        </div>

        <div className="space-y-2">
          <div className="flex items-center justify-between gap-3">
            <p className="text-sm font-medium text-text-primary">{t('settings.webSearchCustomProviders')}</p>
            <Button type="button" variant="ghost" size="sm" icon={<RefreshCw size={14} />} loading={refreshing} onClick={() => void refreshStatus()}>{t('settings.webSearchRefresh')}</Button>
          </div>
          <SettingsRow label={t('settings.webSearchProviderMode')} description={t(PROVIDER_MODE_DESC_KEYS[config.providerMode])}>
            <NexaSelect aria-label={t('settings.webSearchProviderMode')} value={config.providerMode} onChange={event => updateConfig({ ...config, providerMode: event.target.value as WebSearchProviderMode })} className={SELECT_CLASS}>
              {PROVIDER_MODE_OPTIONS.map(mode => <option key={mode} value={mode}>{t(PROVIDER_MODE_LABEL_KEYS[mode])}</option>)}
            </NexaSelect>
          </SettingsRow>
          <p className="text-xs leading-relaxed text-text-tertiary">{t('settings.webSearchCustomProvidersDesc')}</p>
          <p className="text-[11px] leading-relaxed text-text-tertiary">{t('settings.webSearchReorderHint')}</p>
          <DndContext sensors={sensors} collisionDetection={closestCenter} modifiers={[restrictToVerticalAxis, restrictToParentElement]} onDragEnd={reorderProviders} accessibility={{ screenReaderInstructions: { draggable: t('settings.webSearchReorderHint') } }}>
            <SortableContext items={orderedProviders.map(provider => provider.id)} strategy={verticalListSortingStrategy}>
              <div className="divide-y divide-border/50 rounded-lg border border-border/60" data-testid="web-search-sources">
                {orderedProviders.map(provider => <SearchProviderRow
                  key={provider.id}
                  provider={provider}
                  status={statusById.get(provider.id)}
                  expanded={expandedProvider === provider.id}
                  onExpand={() => setExpandedProvider(current => current === provider.id ? null : provider.id)}
                  onChange={patch => updateCustomProvider(provider.preset, patch)}
                />)}
              </div>
            </SortableContext>
          </DndContext>
        </div>

        <details className="group rounded-lg border border-border/60">
          <summary className="flex cursor-pointer list-none items-center justify-between px-3 py-2.5 text-sm font-medium text-text-secondary">{t('settings.advancedSettings')}<ChevronDown size={15} className="group-open:rotate-180" /></summary>
          <div className="divide-y divide-border/50 px-3 pb-2">
            <SettingsRow label={t('settings.webSearchProfile')} description={t(PROFILE_DESC_KEYS[config.providerProfile])}>
              <NexaSelect aria-label={t('settings.webSearchProfile')} value={config.providerProfile} onChange={event => updateConfig({ ...config, providerProfile: event.target.value as WebSearchProviderProfile })} className={SELECT_CLASS}>
                {PROFILE_OPTIONS.map(profile => <option key={profile} value={profile}>{t(PROFILE_LABEL_KEYS[profile])}</option>)}
              </NexaSelect>
            </SettingsRow>
            <SettingsRow label={t('settings.webSearchReranker')} description={t(RERANKER_DESC_KEYS[config.reranker])}>
              <NexaSelect aria-label={t('settings.webSearchReranker')} value={config.reranker} onChange={event => updateConfig({ ...config, reranker: event.target.value as WebSearchReranker })} className={SELECT_CLASS}>
                {RERANKER_OPTIONS.map(reranker => <option key={reranker} value={reranker}>{t(RERANKER_LABEL_KEYS[reranker])}</option>)}
              </NexaSelect>
            </SettingsRow>
          </div>
        </details>

        {status.some(provider => provider.builtIn) && <details className="group rounded-lg border border-border/60">
          <summary className="flex cursor-pointer list-none items-center justify-between px-3 py-2.5 text-sm font-medium text-text-secondary">{t('settings.webSearchProviderStatus')}<ChevronDown size={15} className="group-open:rotate-180" /></summary>
          <div className="divide-y divide-border/50">
            {status.filter(provider => provider.builtIn).map(provider => <div key={provider.id} className="flex min-w-0 items-center justify-between gap-3 px-3 py-2">
              <div className="min-w-0 text-xs">
                <p className="font-medium text-text-primary">{provider.label}</p>
                <p className="mt-0.5 text-text-tertiary">{t(provider.enabledByProfile ? 'settings.webSearchProviderActive' : 'settings.webSearchProviderFallback')}</p>
                {provider.lastErrorCode && <p className="mt-0.5 text-text-tertiary">{t('settings.webSearchLastError')}: {provider.lastErrorCode}{provider.nextRetrySeconds ? `, ${t('settings.webSearchRetryIn', { seconds: provider.nextRetrySeconds })}` : ''}</p>}
              </div>
              <span className={`shrink-0 rounded-full px-2 py-0.5 text-[11px] ${healthClass(provider.health)}`}>{t(HEALTH_LABEL_KEYS[provider.health])}</span>
            </div>)}
          </div>
        </details>}
        <div data-settings-actions className="flex justify-end"><Button size="sm" onClick={onSave} disabled={loading}>{loading ? '...' : t('common.save')}</Button></div>
      </div>
    </Section>
  );
}
