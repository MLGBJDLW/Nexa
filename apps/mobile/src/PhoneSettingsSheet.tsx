import { useEffect, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Check, ChevronRight, Loader2, Plus, RefreshCw, Search, X } from 'lucide-react';
import { useTranslation } from '../../desktop/src/i18n';
import { PROVIDER_PRESETS } from '../../desktop/src/lib/providerPresets';
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

const presets = PROVIDER_PRESETS.filter(preset => !preset.runtime && preset.requiresApiKey && !['bedrock','vertex','github_copilot_api'].includes(preset.provider));
const presetFor = (record?: PhoneConnection) => presets.find(preset => preset.provider === record?.provider && preset.baseUrl === record.baseUrl)
  ?? presets.find(preset => preset.provider === record?.provider) ?? presets.find(preset => preset.id === 'openai')!;

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
  const [model, setModel] = useState(initialRecord?.model ?? initial.models[0]?.id ?? '');
  const [baseUrl, setBaseUrl] = useState(initialRecord?.baseUrl ?? initial.baseUrl);
  const [apiKey, setApiKey] = useState('');
  const [error, setError] = useState('');
  const [search, setSearch] = useState('');
  const [expanded, setExpanded] = useState<Record<string, boolean>>({ connections: true });
  const [busy, setBusy] = useState(false);
  const [removeTarget, setRemoveTarget] = useState<PhoneConnection | null>(null);
  const preset = presets.find(item => item.id === provider)!;
  const loadRecord = (record?: PhoneConnection) => {
    const preset = presetFor(record);
    setSelectedId(record?.id ?? ''); setName(record?.name ?? ''); setProvider(preset.id);
    setModel(record?.model ?? preset.models[0]?.id ?? ''); setBaseUrl(record?.baseUrl ?? preset.baseUrl); setApiKey(''); setError('');
  };
  // Preserve an in-progress form while background sync updates the library.
  useEffect(() => { if (open) loadRecord(state.connections?.find(record => record.id === state.activeConnectionId)); }, [open]);
  const action = async (run: () => Promise<void>) => { setError(''); setBusy(true); try { await run(); await onChanged(); } catch (error) { setError(String(error)); } finally { setBusy(false); } };
  const groups = {
    connections: [t('remote.mobileConnections'), t('settings.provider'), t('remote.model'), t('settings.apiKey'), t('settings.baseUrl'), ...presets.map(item => item.name)],
    sync: [t('remote.mobileConnectionSync'), t('remote.mobileAllowComputerControl'), ...state.hosts.map(hostLabel)],
    permissions: [t('remote.mobilePermissions'), t('remote.mobileNotifications'), t('remote.mobileAccessibility'), t('remote.mobileFloatingAssistant'), t('remote.mobileBatterySettings')],
    speech: [t('remote.mobileSpeechSettings'), t('settings.speechToText'), t('remote.mobileSpeechOutput'), t('settings.apiKey'), 'microphone voice audio 语音 麦克风'],
    web: [t('settings.webSearchTitle'), 'search fetch DeepSeek 搜索 联网'],
    storage: [t('settings.storageSection'), t('settings.storageManageChats'), 'storage cache space 存储 缓存 空间'],
    language: [t('remote.language'), ...availableLocales.map(item => item.name)],
  };
  type GroupId = keyof typeof groups;
  const query = search.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const matches = (id: GroupId) => (id !== 'sync' || state.hosts.length > 0) && query.every(word => groups[id].join(' ').toLocaleLowerCase().includes(word));
  const group = (id: GroupId, children: ReactNode) => <details className="mobile-settings-group" hidden={!matches(id)} open={query.length > 0 || !!expanded[id]} onToggle={event => { if (!query.length) { const next = event.currentTarget.open; setExpanded(current => current[id] === next ? current : { ...current, [id]: next }); } }}>
    <summary>{groups[id][0]}<ChevronRight size={17}/></summary><div className="mobile-settings-group-body">{children}</div>
  </details>;
  return <>
    <MobileDialog open={open} title={t('remote.mobileSettings')} onClose={onClose}>
      <label className="mobile-settings-search"><Search size={18}/><input type="search" aria-label={t('remote.mobileSettingsSearch')} placeholder={t('remote.mobileSettingsSearch')} value={search} onChange={event => setSearch(event.target.value)}/>{search && <button type="button" className="mobile-icon-button" aria-label={t('common.clear')} onClick={() => setSearch('')}><X size={16}/></button>}</label>
      {!(Object.keys(groups) as GroupId[]).some(matches) && <p role="status" className="mobile-section-hint">{t('remote.mobileSettingsNoResults')}</p>}
      {error && <p role="alert" className="mobile-error">{error}</p>}
      {group('connections', <>
      {!!state.connections?.length && <label className="mobile-connection-picker">{t('remote.mobileConnections')}<select aria-label={t('remote.mobileConnections')} value={selectedId} disabled={busy} onChange={event => {
        const record = state.connections?.find(record => record.id === event.target.value);
        if (record) void action(async () => { await invoke('mobile_connection', { action: 'select', id: record.id }); loadRecord(record); });
      }}><option value="" disabled>{t('remote.mobileAddConnection')}</option>{state.connections.map(record => <option key={record.id} value={record.id}>{record.name} · {record.model}</option>)}</select></label>}
      <button className="mobile-add-connection" disabled={busy} onClick={() => loadRecord()}><Plus size={16}/>{t('remote.mobileAddConnection')}</button>
      <form className="mobile-settings-form" onSubmit={event => { event.preventDefault(); void action(async () => {
        await invoke('mobile_save_settings', { settings: { connectionId: selectedId || null, providerType: preset.provider, baseUrl: baseUrl.trim() || null, model: model.trim() }, apiKey, name: name.trim() || model.trim(), createNew: !selectedId });
        setApiKey(''); onClose();
      }); }}>
        <label>{t('settings.providerName')}<input aria-label={t('settings.providerName')} value={name} onChange={event => setName(event.target.value)} placeholder={model}/></label>
        <label>{t('settings.provider')}<NexaCombobox ariaLabel={t('settings.provider')} value={provider} disabled={busy} options={presets.map(item => ({value:item.id,label:item.name,description:item.baseUrl,keywords:[item.provider]}))} searchPlaceholder={t('settings.modelSearchPlaceholder')} emptyLabel={t('settings.modelSearchNoResults')} onValueChange={value => { const next = presets.find(item => item.id === value)!; setProvider(next.id); setBaseUrl(next.baseUrl); setModel(next.models[0]?.id ?? ''); setApiKey(''); }}/></label>
        <label>{t('remote.model')}<input aria-label={t('remote.model')} list="phone-models" value={model} onChange={event => setModel(event.target.value)} autoCapitalize="none" spellCheck={false}/></label><datalist id="phone-models">{preset.models.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</datalist>
        <label>{t('settings.apiKey')}<input aria-label={t('settings.apiKey')} type="password" value={apiKey} onChange={event => setApiKey(event.target.value)} placeholder={selectedId ? '••••••••' : ''} autoComplete="off"/></label>
        <details className="mobile-advanced" open={baseUrl !== preset.baseUrl || undefined}><summary>{t('remote.advancedOptions')}</summary><label>{t('settings.baseUrl')}<input aria-label={t('settings.baseUrl')} type="url" value={baseUrl} onChange={event => setBaseUrl(event.target.value)} autoCapitalize="none" spellCheck={false}/></label></details>
        <p className="mobile-section-hint">{t('remote.mobileCredentials')}</p><button className="mobile-primary" disabled={busy}>{busy && <Loader2 size={16} className="animate-spin"/>}{t('common.save')}</button>
      </form>
      {selectedId && <button className="mobile-forget" disabled={busy} onClick={() => setRemoveTarget(state.connections?.find(record => record.id === selectedId) ?? null)}>{t('remote.mobileRemoveConnection')}</button>}
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
      {group('speech', <PhoneSpeechSettings open={open && matches('speech') && (query.length > 0 || !!expanded.speech)}/>)}
      {group('web', open && <PhoneWebSettings connections={state.connections}/>)}
      {group('storage', open && matches('storage') && (query.length > 0 || expanded.storage) && <StorageUsageCard mobile service={phoneStorage}/>)}
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
