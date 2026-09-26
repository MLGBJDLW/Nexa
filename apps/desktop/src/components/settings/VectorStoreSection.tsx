import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Cloud, RefreshCw, Save, Square, Zap } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { Section } from './SettingsSection';
import { NexaSelect } from '../ui/overlay';
import { Input } from '../ui/Input';
import { Button } from '../ui/Button';

interface VectorConfig {
  mode: 'local' | 'cloud' | 'hybrid';
  provider: 'qdrant' | 'pinecone' | 'dashvector' | 'milvus' | 'tencent';
  endpoint: string; apiKey: string; collectionPrefix: string; database: string; account: string;
}
interface SyncStatus {
  localVectors: number; uploadedVectors: number; pendingUploads: number; pendingDeletes: number;
  syncing: boolean; paused: boolean; lastError: string | null;
}
const DEFAULT_CONFIG: VectorConfig = { mode: 'local', provider: 'qdrant', endpoint: '', apiKey: '', collectionPrefix: 'nexa', database: 'default', account: 'root' };
const PROVIDERS = [
  ['qdrant', 'Qdrant / Qdrant Cloud'], ['pinecone', 'Pinecone'], ['dashvector', 'Alibaba DashVector'],
  ['milvus', 'Milvus / Zilliz Cloud'], ['tencent', 'Tencent VectorDB'],
] as const;

export function VectorStoreSection() {
  const { t } = useTranslation();
  const [config, setConfig] = useState<VectorConfig>(DEFAULT_CONFIG);
  const [saved, setSaved] = useState<VectorConfig>(DEFAULT_CONFIG);
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState('');
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const dirty = JSON.stringify(config) !== JSON.stringify(saved);
  useEffect(() => {
    let disposed = false;
    void invoke<VectorConfig | null>('get_vector_store_config_cmd').then(value => {
      if (!disposed) {setConfig(value ?? DEFAULT_CONFIG); setSaved(value ?? DEFAULT_CONFIG);}
    }).catch(cause => {if (!disposed) setError(String(cause));}).finally(() => {if (!disposed) setLoading(false);});
    return () => {disposed = true;};
  }, []);
  useEffect(() => {
    if (saved.mode === 'local') {setStatus(null); return;}
    let disposed = false;
    let pending = false;
    const refresh = async () => {
      if (pending || disposed || document.hidden) return;
      pending = true;
      try {const next = await invoke<SyncStatus>('get_vector_store_status_cmd'); if (!disposed) setStatus(next);}
      catch (cause) {if (!disposed) setError(String(cause));}
      finally {pending = false;}
    };
    void refresh();
    const timer = setInterval(() => {void refresh();}, 3000);
    return () => {disposed = true; clearInterval(timer);};
  }, [saved]);
  const update = (patch: Partial<VectorConfig>) => {setConfig(value => ({...value, ...patch})); setNotice(''); setError('');};
  const action = async (kind: 'save' | 'test' | 'sync' | 'pause') => {
    setBusy(kind); setError(''); setNotice('');
    try {
      if (kind === 'save') {await invoke('save_vector_store_config_cmd', {config}); setSaved(config); setNotice(t('settings.privacySaved'));}
      if (kind === 'test') {await invoke('test_vector_store_connection_cmd', {config}); setNotice(t('settings.embeddingTestSuccess'));}
      if (kind === 'sync') await invoke('sync_vector_store_cmd');
      if (kind === 'pause') await invoke('cancel_vector_store_sync_cmd');
      setStatus(await invoke<SyncStatus>('get_vector_store_status_cmd'));
    } catch (cause) {setError(String(cause));}
    finally {setBusy('');}
  };
  return <Section icon={<Cloud size={20} />} title={t('settings.vectorStoreSection')} collapsible defaultOpen={false}>
    <div className="min-w-0 space-y-3" data-testid="vector-store-settings">
      <p className="text-xs leading-5 text-text-tertiary">{t('settings.vectorStoreLocalDefault')}</p>
      <NexaSelect aria-label={t('settings.vectorStoreMode')} value={config.mode} disabled={loading || busy === 'save'}
        onChange={event => update({mode: event.target.value as VectorConfig['mode']})}
        className="h-10 w-full rounded-md border border-border bg-surface-2 px-3 text-sm">
        <option value="local">{t('settings.vectorStoreLocal')}</option>
        <option value="cloud">{t('settings.vectorStoreCloud')}</option>
        <option value="hybrid">{t('settings.vectorStoreHybrid')}</option>
      </NexaSelect>
      {config.mode !== 'local' && <div className="space-y-3 rounded-lg border border-border bg-surface-2 p-4">
        <label className="block space-y-1 text-sm">{t('settings.provider')}
          <NexaSelect aria-label={t('settings.vectorStoreProvider')} value={config.provider} onChange={event => update({provider: event.target.value as VectorConfig['provider'], endpoint: '', apiKey: ''})}
            className="h-10 w-full rounded-md border border-border bg-surface-1 px-3">{PROVIDERS.map(([value,label]) => <option key={value} value={value}>{label}</option>)}</NexaSelect>
        </label>
        <label className="block space-y-1 text-sm">{t('settings.vectorStoreEndpoint')}<Input aria-label={t('settings.vectorStoreEndpoint')} value={config.endpoint} onChange={event => update({endpoint: event.target.value})} placeholder="https://" /></label>
        <label className="block space-y-1 text-sm">{t('settings.embeddingApiKey')}<Input aria-label={t('settings.vectorStoreKey')} type="password" value={config.apiKey} onChange={event => update({apiKey: event.target.value})} /></label>
        <label className="block space-y-1 text-sm">{t('settings.vectorStorePrefix')}<Input aria-label={t('settings.vectorStorePrefix')} value={config.collectionPrefix} maxLength={12} onChange={event => update({collectionPrefix: event.target.value})} /></label>
        {(config.provider === 'milvus' || config.provider === 'tencent') && <label className="block space-y-1 text-sm">{t('settings.vectorStoreDatabase')}<Input aria-label={t('settings.vectorStoreDatabase')} value={config.database} onChange={event => update({database: event.target.value})} /></label>}
        {config.provider === 'tencent' && <label className="block space-y-1 text-sm">{t('settings.vectorStoreAccount')}<Input aria-label={t('settings.vectorStoreAccount')} value={config.account} onChange={event => update({account: event.target.value})} /></label>}
        <p className="text-xs leading-5 text-text-tertiary">{t('settings.vectorStoreSyncHelp')}</p>
        <Button variant="secondary" size="sm" icon={<Zap size={14} />} disabled={!!busy || !config.endpoint.trim()} onClick={() => {void action('test');}}>{t('settings.embeddingTestConnection')}</Button>
      </div>}
      {config.mode === 'hybrid' && <p className="text-xs leading-5 text-text-tertiary">{t('settings.vectorStoreFusionHelp')}</p>}
      {status && saved.mode !== 'local' && <div role="status" data-testid="vector-store-sync-status" className="space-y-1 text-xs text-text-secondary">
        <p>{t('settings.vectorStoreCoverage', {uploaded: String(status.uploadedVectors), total: String(status.localVectors), pending: String(status.pendingUploads), deletes: String(status.pendingDeletes)})}</p>
        {status.paused && <p>{t('settings.vectorStorePaused')}</p>}
        {status.lastError && <p className="break-words text-warning">{status.lastError}</p>}
      </div>}
      {error && <p role="alert" className="break-words text-xs text-danger">{error}</p>}
      {notice && <p role="status" className="text-xs text-success">{notice}</p>}
      <div className="flex flex-wrap gap-2">
        <Button size="sm" icon={<Save size={14} />} disabled={loading || !!busy} loading={busy === 'save'} onClick={() => {void action('save');}}>{t('settings.embeddingSave')}</Button>
        {saved.mode !== 'local' && <Button size="sm" variant="secondary" icon={<RefreshCw size={14} />} disabled={dirty || !!busy || status?.syncing} onClick={() => {void action('sync');}}>{t('settings.vectorStoreSync')}</Button>}
        {(status?.syncing || busy === 'sync') && <Button size="sm" variant="secondary" icon={<Square size={12} />} onClick={() => {void action('pause');}}>{t('settings.vectorStorePause')}</Button>}
      </div>
    </div>
  </Section>;
}
