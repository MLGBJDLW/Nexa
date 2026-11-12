import { useEffect, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Check, ChevronRight, Loader2, Plus, RefreshCw, Search, X } from 'lucide-react';
import { useTranslation } from '../../desktop/src/i18n';
import { findProviderPreset, PROVIDER_PRESETS, type ProviderPreset } from '../../desktop/src/lib/providerPresets';
import { selectImplicitDefault } from '../../desktop/src/lib/modelCatalog';
import { MobileDialog } from '../../desktop/src/features/remote/MobileChatShell';
import { NexaCombobox } from '../../desktop/src/components/ui/overlay/Combobox';
import type { ConnectionSyncSummary, MobileState, PhoneConnection } from './MobileApp';
import { hostLabel, type NativeHost } from './nativeClient';
import { PhoneWebSettings } from './PhoneWebSettings';
import { PhoneSpeechSettings } from './PhoneSpeechSettings';
import { StorageUsageCard } from '../../desktop/src/components/settings/StorageUsageCard';
import type { StorageService } from '../../desktop/src/lib/api';

const phoneStorage: StorageService = {
  usage: () => invoke('mobile_storage', { action: 'usage' }),
  page: (scope, cursor) => invoke('mobile_storage', { action: 'page', scope, cursor }),
  archive: ids => invoke('mobile_storage', { action: 'archive', ids }),
  remove: ids => invoke('mobile_storage', { action: 'remove', ids }),
};

function PhoneStorageSettings() {
  const { t } = useTranslation();
  const [locations, setLocations] = useState<{data: string; cache: string} | null>(null);
  const [error, setError] = useState('');
  useEffect(() => {
    let active = true;
    void invoke<{data: string; cache: string}>('mobile_storage', { action: 'locations' })
      .then(value => { if (active) setLocations(value); })
      .catch(error => { if (active) setError(String(error)); });
    return () => { active = false; };
  }, []);
  return <><StorageUsageCard mobile service={phoneStorage}/>
    <section className="mobile-settings-section"><h3>{t('settings.storageSection')}</h3>
      {error && <p role="status" className="mobile-error">{error}</p>}
      {locations && <dl className="text-xs break-all"><dt>{t('settings.storageInternalState')}</dt><dd className="py-2 font-mono">{locations.data}</dd><dt>{t('settings.storageRuntime')}</dt><dd className="py-2 font-mono">{locations.cache}</dd></dl>}
    </section></>;
}

const presets = PROVIDER_PRESETS.filter(preset => !preset.runtime && preset.requiresApiKey && !['bedrock','vertex','github_copilot_api'].includes(preset.provider));
type PhonePreset = Pick<ProviderPreset, 'id' | 'name' | 'provider' | 'baseUrl' | 'models' | 'retired'>;
function presetFor(record?: PhoneConnection): PhonePreset {
  if (!record) return presets.find(preset => preset.id === 'openai')!;
  const matched = findProviderPreset(record);
  if (matched && matched.provider === record.provider && presets.includes(matched)) return matched;
  // Keep the account's actual adapter and endpoint when an older or synced
  // connection has no equivalent in this phone's new-provider picker.
  return { id: `saved:${record.id}`, name: record.provider, provider: record.provider, baseUrl: record.baseUrl ?? '', models: [] };
}

function SettingsSearch({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  const { t } = useTranslation();
  const field = useRef<HTMLInputElement>(null);
  const [compact, setCompact] = useState(false);
  const scrollWhileEditing = useRef(false);
  useEffect(() => {
    const sheet = field.current?.closest('dialog');
    if (!sheet) return;
    let anchor = sheet.scrollTop;
    const allowGesture = () => { scrollWhileEditing.current = true; };
    const outsideField = (event: PointerEvent) => {
      if (!(event.target as Element).closest('.mobile-settings-search')) allowGesture();
    };
    const keyScroll = (event: KeyboardEvent) => {
      if (event.key === 'PageDown' || event.key === 'PageUp') allowGesture();
    };
    const onScroll = () => {
      const top = sheet.scrollTop;
      // Focusing a sticky field, opening the keyboard, and filtering results
      // can all scroll the dialog. Only a new scroll gesture may end editing.
      if (sheet.ownerDocument.activeElement === field.current && !scrollWhileEditing.current) {
        anchor = top;
        return;
      }
      if (top <= 12) setCompact(false);
      if (top > 64 && top - anchor >= 24) {
        field.current?.blur();
        setCompact(true);
      }
      if (top < anchor || top - anchor >= 24) anchor = top;
    };
    sheet.addEventListener('scroll', onScroll, { passive: true });
    sheet.addEventListener('wheel', allowGesture, { passive: true });
    sheet.addEventListener('touchmove', allowGesture, { passive: true });
    sheet.addEventListener('pointerdown', outsideField);
    sheet.addEventListener('keydown', keyScroll);
    return () => {
      sheet.removeEventListener('scroll', onScroll);
      sheet.removeEventListener('wheel', allowGesture);
      sheet.removeEventListener('touchmove', allowGesture);
      sheet.removeEventListener('pointerdown', outsideField);
      sheet.removeEventListener('keydown', keyScroll);
    };
  }, []);
  const focus = () => { scrollWhileEditing.current = false; setCompact(false); field.current?.focus({ preventScroll: true }); };
  const label = t('remote.mobileSettingsSearch');
  return <div className="mobile-settings-search" data-compact={compact}>
    <button type="button" className="mobile-icon-button" aria-label={label} aria-expanded={!compact}
      tabIndex={compact ? 0 : -1} onClick={focus}><Search size={18}/></button>
    <input ref={field} type="search" aria-label={label} placeholder={label} value={value}
      tabIndex={compact ? -1 : 0} aria-hidden={compact || undefined} onFocus={() => { scrollWhileEditing.current = false; setCompact(false); }}
      onChange={event => { scrollWhileEditing.current = false; onChange(event.target.value); }} />
    {value && !compact && <button type="button" className="mobile-icon-button" aria-label={t('common.clear')}
      onClick={() => { onChange(''); focus(); }}><X size={16}/></button>}
  </div>;
}

export default function PhoneSettingsSheet({ open, state, host, syncBusy, syncErrors, onSync, onClose, onChanged, onForgot }: {
  open: boolean; state: MobileState; host?: NativeHost; syncBusy: Record<string, boolean>; syncErrors: Record<string, string>;
  onSync: (id: string) => Promise<ConnectionSyncSummary>; onClose: () => void; onChanged: () => Promise<void>; onForgot: () => void;
}) {
  const { t, locale, setLocale, availableLocales } = useTranslation();
  const initialRecord = state.connections?.find(record => record.id === state.activeConnectionId);
  const initial = presetFor(initialRecord);
  const [selectedId, setSelectedId] = useState(initialRecord?.id ?? '');
  const [name, setName] = useState(initialRecord?.name ?? '');
  const [provider, setProvider] = useState(initial.id);
  const [model, setModel] = useState(initialRecord?.model ?? selectImplicitDefault(initial.models)?.id ?? '');
  const [baseUrl, setBaseUrl] = useState(initialRecord ? initialRecord.baseUrl ?? '' : initial.baseUrl);
  const [apiKey, setApiKey] = useState('');
  const [error, setError] = useState('');
  const [search, setSearch] = useState('');
  const [tab, setTab] = useState<'general' | 'data_privacy'>('general');
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [searchCollapsed, setSearchCollapsed] = useState<Record<string, boolean>>({});
  const [editingConnection, setEditingConnection] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [removeTarget, setRemoveTarget] = useState<PhoneConnection | null>(null);
  const savedPreset = presetFor(state.connections?.find(record => record.id === selectedId));
  const choices: PhonePreset[] = presets.filter(item => !item.retired || item.id === savedPreset.id);
  if (!choices.some(item => item.id === savedPreset.id)) choices.push(savedPreset);
  const preset = choices.find(item => item.id === provider) ?? savedPreset;
  const loadRecord = (record?: PhoneConnection) => {
    const preset = presetFor(record);
    setSelectedId(record?.id ?? ''); setName(record?.name ?? ''); setProvider(preset.id);
    setModel(record?.model ?? selectImplicitDefault(preset.models)?.id ?? ''); setBaseUrl(record ? record.baseUrl ?? '' : preset.baseUrl); setApiKey(''); setError('');
  };
  // Preserve an in-progress form while background sync updates the library.
  useEffect(() => {
    if (open) {
      loadRecord(state.connections?.find(record => record.id === state.activeConnectionId));
      setEditingConnection(null); setExpanded({}); setSearchCollapsed({}); setSearch(''); setTab('general');
    }
  }, [open]);
  const action = async (run: () => Promise<void>) => { setError(''); setBusy(true); try { await run(); await onChanged(); } catch (error) { setError(String(error)); } finally { setBusy(false); } };
  const groups = {
    connections: [t('remote.mobileConnections'), t('settings.provider'), t('remote.model'), t('settings.apiKey'), t('settings.baseUrl'), 'LLM', ...choices.map(item => item.name), ...(state.connections ?? []).map(item => `${item.name} ${item.model}`)],
    sync: [t('remote.mobileConnectionSync'), t('remote.mobileAllowComputerControl'), ...state.hosts.map(hostLabel)],
    permissions: [t('remote.mobilePermissions'), t('remote.mobileNotifications'), t('remote.mobileAccessibility'), t('remote.mobileFloatingAssistant'), t('remote.mobileBatterySettings')],
    speech: [t('remote.mobileSpeechSettings'), t('settings.speechToText'), t('remote.mobileSpeechOutput'), t('settings.apiKey'), 'microphone voice audio 语音 麦克风'],
    web: [t('settings.webSearchTitle'), 'search fetch DeepSeek 搜索 联网'],
    storage: [t('settings.storageSection'), t('settings.tabDataPrivacy'), t('settings.storageManageChats'), 'storage cache space 存储 缓存 空间'],
    language: [t('remote.language'), ...availableLocales.map(item => item.name)],
  };
  type GroupId = keyof typeof groups;
  const query = search.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const matches = (id: GroupId) => (query.length > 0 || (id === 'storage') === (tab === 'data_privacy')) && (id !== 'sync' || state.hosts.length > 0) && query.every(word => groups[id].join(' ').toLocaleLowerCase().includes(word));
  const isExpanded = (id: GroupId) => query.length ? !searchCollapsed[id] : (expanded[id] ?? id === 'storage');
  const editorOpen = !selectedId || (editingConnection ?? query.length > 0);
  const group = (id: GroupId, children: ReactNode) => <details className="mobile-settings-group" hidden={!matches(id)} open={isExpanded(id)}>
    <summary onClick={event => { event.preventDefault(); (query.length ? setSearchCollapsed : setExpanded)(current => ({ ...current, [id]: !current[id] })); }}>
      <span>{groups[id][0]}{id === 'connections' && initialRecord && <small>{initialRecord.model}</small>}</span>
      <ChevronRight size={17}/>
    </summary><div className="mobile-settings-group-body">{children}</div>
  </details>;
  return <>
    <MobileDialog open={open} title={t('remote.mobileSettings')} onClose={onClose}>
      <nav className="mobile-settings-tabs" aria-label={t('remote.mobileSettings')}>
        <button type="button" aria-pressed={tab === 'general'} onClick={() => { setTab('general'); setSearch(''); }}>{t('remote.mobileSettings')}</button>
        <button type="button" aria-pressed={tab === 'data_privacy'} onClick={() => { setTab('data_privacy'); setSearch(''); }}>{t('settings.tabDataPrivacy')}</button>
      </nav>
      <SettingsSearch value={search} onChange={value => { setSearch(value); setSearchCollapsed({}); }}/>
      {!(Object.keys(groups) as GroupId[]).some(matches) && <p role="status" className="mobile-section-hint">{t('remote.mobileSettingsNoResults')}</p>}
      {error && <p role="alert" className="mobile-error">{error}</p>}
      {group('connections', <>
      {!!state.connections?.length && <label className="mobile-connection-picker">{t('remote.mobileConnections')}<select aria-label={t('remote.mobileConnections')} value={selectedId} disabled={busy} onChange={event => {
        const record = state.connections?.find(record => record.id === event.target.value);
        if (record) void action(async () => { await invoke('mobile_connection', { action: 'select', id: record.id }); loadRecord(record); });
      }}><option value="" disabled>{t('remote.mobileAddConnection')}</option>{state.connections.map(record => <option key={record.id} value={record.id}>{record.name} · {record.model}</option>)}</select></label>}
      <button className="mobile-add-connection" disabled={busy} onClick={() => { loadRecord(); setEditingConnection(true); }}><Plus size={16}/>{t('remote.mobileAddConnection')}</button>
      <details className="mobile-connection-editor" open={editorOpen}>
      <summary hidden={!selectedId} onClick={event => { event.preventDefault(); setEditingConnection(!editorOpen); }}>
        {t('common.edit')}<ChevronRight size={16}/>
      </summary>
      <form className="mobile-settings-form" onSubmit={event => { event.preventDefault(); void action(async () => {
        await invoke('mobile_save_settings', { settings: { connectionId: selectedId || null, providerType: preset.provider, baseUrl: baseUrl.trim() || null, model: model.trim() }, apiKey, name: name.trim() || model.trim(), createNew: !selectedId });
        setApiKey(''); onClose();
      }); }}>
        <label>{t('settings.providerName')}<input aria-label={t('settings.providerName')} value={name} onChange={event => setName(event.target.value)} placeholder={model}/></label>
        <label>{t('settings.provider')}<NexaCombobox ariaLabel={t('settings.provider')} value={provider} disabled={busy} options={choices.map(item => ({value:item.id,label:item.name,description:item.baseUrl,keywords:[item.provider]}))} searchPlaceholder={t('settings.modelSearchPlaceholder')} emptyLabel={t('settings.modelSearchNoResults')} onValueChange={value => { const next = choices.find(item => item.id === value)!; setProvider(next.id); setBaseUrl(next.baseUrl); setModel(selectImplicitDefault(next.models)?.id ?? ''); setApiKey(''); }}/></label>
        <label>{t('remote.model')}<input aria-label={t('remote.model')} list="phone-models" value={model} required pattern=".*\S.*" onChange={event => setModel(event.target.value)} autoCapitalize="none" spellCheck={false}/></label><datalist id="phone-models">{preset.models.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</datalist>
        <label>{t('settings.apiKey')}<input aria-label={t('settings.apiKey')} type="password" value={apiKey} onChange={event => setApiKey(event.target.value)} placeholder={selectedId ? '••••••••' : ''} autoComplete="off"/></label>
        <details className="mobile-advanced" open={baseUrl !== preset.baseUrl || undefined}><summary>{t('remote.advancedOptions')}</summary><label>{t('settings.baseUrl')}<input aria-label={t('settings.baseUrl')} type="url" value={baseUrl} onChange={event => setBaseUrl(event.target.value)} autoCapitalize="none" spellCheck={false}/></label></details>
        <p className="mobile-section-hint">{t('remote.mobileCredentials')}</p><button className="mobile-primary" disabled={busy}>{busy && <Loader2 size={16} className="animate-spin"/>}{t('common.save')}</button>
      </form>
      {selectedId && <button className="mobile-forget" disabled={busy} onClick={() => setRemoveTarget(state.connections?.find(record => record.id === selectedId) ?? null)}>{t('remote.mobileRemoveConnection')}</button>}
      </details>
      </>)}
      {!!state.hosts.length && group('sync', <section className="mobile-settings-section"><h3>{t('remote.mobileConnectionSync')}</h3><p className="mobile-section-hint">{t('remote.mobileSyncTrustedHint')}</p>
        {state.hosts.map(computer => {
          const summary = state.connectionSync?.[computer.id];
          return <div className="mobile-sync-host" key={computer.id}><strong>{hostLabel(computer)}</strong>
            <label className="mobile-overlay-toggle"><span>{t('remote.mobileAllowComputerControl')}</span><input type="checkbox" checked={state.capabilities.remoteControlHost === computer.id}
              disabled={busy || !computer.manifest.capabilities?.includes('phone-control-v1')} onChange={event => { const enabled=event.target.checked; void action(async () => { await invoke('mobile_phone_control',{hostId:computer.id,enabled}); }); }}/></label>
            <p className="mobile-section-hint">{t('remote.mobileComputerControlHint')}</p>
            {summary && <p className="mobile-section-hint">{summary.supported ? t('remote.mobileSyncSummary', { count: summary.connections }) : t('remote.mobileSyncUnsupported')}</p>}
            {!!summary?.conflicts && <p className="mobile-section-hint">{t('remote.mobileSyncConflicts', { count: summary.conflicts })}</p>}
            {!!summary?.deviceOnly && <p className="mobile-section-hint">{t('remote.mobileSyncDeviceOnly', { count: summary.deviceOnly })}</p>}
            {syncErrors[computer.id] && <p className="mobile-error" role="status">{syncErrors[computer.id]}</p>}
            <button disabled={!!syncBusy[computer.id]} onClick={() => void action(async () => { await onSync(computer.id); })}>{syncBusy[computer.id] ? t('remote.mobileSyncing') : t('remote.mobileSyncNow')}{syncBusy[computer.id] ? <Loader2 size={16} className="animate-spin"/> : <RefreshCw size={16}/>}</button>
          </div>;
        })}
      </section>)}
      {group('permissions', <section className="mobile-settings-section"><h3>{t('remote.mobilePermissions')}</h3><p className="mobile-section-hint">{t('remote.mobileControlHint')}</p>
        <p role="status" className="mobile-permission-status">{t(state.capabilities.accessibility ? 'remote.mobileAccessConnected' : state.capabilities.accessibilityEnabled ? 'remote.mobileAccessDisconnected' : 'remote.mobileAccessDisabled')}</p>
        <button onClick={() => void action(async () => { await invoke('mobile_permission', { permission: 'notifications' }); })}>{t('remote.mobileNotifications')}{state.capabilities.notifications ? <Check size={17}/> : <ChevronRight size={17}/>}</button>
        <button onClick={() => void action(async () => { await invoke('mobile_permission', { permission: 'accessibility' }); })}>{t('remote.mobileAccessibility')}{state.capabilities.accessibility ? <Check size={17}/> : <ChevronRight size={17}/>}</button>
        <label className="mobile-overlay-toggle"><span>{t('remote.mobileFloatingAssistant')}</span><input type="checkbox" checked={state.capabilities.assistantOverlay ?? false} disabled={busy || (!state.capabilities.accessibility && !state.capabilities.assistantOverlay)} onChange={event => { const enabled = event.target.checked; void action(async () => { await invoke('mobile_overlay', { enabled }); }); }}/></label><p className="mobile-section-hint">{t('remote.mobileFloatingAssistantHint')}</p>
        {!state.capabilities.accessibility && <div className="mobile-permission-help"><p>{t(state.capabilities.accessibilityEnabled ? 'remote.mobileReconnectAccessHint' : 'remote.mobileEnableAccessHint')}</p>{state.capabilities.restrictedSettingsMayApply && <p>{t('remote.mobileRestrictedSettingsHint')}</p>}<button onClick={() => void action(async () => { await invoke('mobile_permission', { permission: 'app_info' }); })}>{t('remote.mobileAppInfo')}<ChevronRight size={17}/></button></div>}
        <details className="mobile-permission-help"><summary>{t('remote.mobileBackgroundHelp')}</summary><p>{t('remote.mobileBackgroundHint')}</p>{/samsung/i.test(state.capabilities.manufacturer ?? '') && <p>{t('remote.mobileSamsungBatteryHint')}</p>}{/xiaomi|redmi|poco|oppo|vivo|oneplus|huawei|honor/i.test(state.capabilities.manufacturer ?? '') && <p>{t('remote.mobileVendorBatteryHint')}</p>}{state.capabilities.backgroundRestricted && <p>{t('remote.mobileBackgroundRestricted')}</p>}<button onClick={() => void action(async () => { await invoke('mobile_permission', { permission: 'battery' }); })}>{t('remote.mobileBatterySettings')}<ChevronRight size={17}/></button></details>
        <button disabled={busy} onClick={() => void action(onChanged)}>{t('remote.mobileRefreshPermissions')}<RefreshCw size={17}/></button>
      </section>)}
      {group('speech', <PhoneSpeechSettings open={open && matches('speech') && isExpanded('speech')}/>)}
      {group('web', open && <PhoneWebSettings connections={state.connections}/>)}
      {group('storage', open && matches('storage') && isExpanded('storage') && <PhoneStorageSettings/>)}
      {group('language', <label className="mobile-language">{t('remote.language')}<select aria-label={t('remote.language')} value={locale} onChange={event => setLocale(event.target.value as typeof locale)}>{availableLocales.map(item => <option key={item.code} value={item.code}>{item.name}</option>)}</select></label>)}
      {host && <button className="mobile-forget" onClick={() => void action(async () => { await invoke('mobile_remote', { hostId: host.id, action: 'forget' }); onForgot(); onClose(); })}>{t('remote.forget')}</button>}
      <p className="mobile-version">Nexa Mobile {state.version}</p>
    </MobileDialog>
    <MobileDialog open={!!removeTarget} title={t('remote.mobileRemoveConnection')} onClose={() => setRemoveTarget(null)}>
      <p>{removeTarget?.name}</p><p className="mobile-section-hint">{t('remote.mobileRemoveConnectionHint')}</p>
      <div className="mobile-confirm-actions"><button onClick={() => setRemoveTarget(null)}>{t('common.cancel')}</button><button className="mobile-primary" disabled={busy} onClick={() => void action(async () => { if (removeTarget) await invoke('mobile_connection', { action: 'remove', id: removeTarget.id }); setRemoveTarget(null); onClose(); })}>{t('common.delete')}</button></div>
    </MobileDialog>
  </>;
}
