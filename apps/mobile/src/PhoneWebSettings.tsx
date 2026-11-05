import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { WebSearchSettingsPanel } from '../../desktop/src/components/settings/WebSearchSettingsPanel';
import type { AppConfig, WebSearchConfig, WebSearchProviderStatus } from '../../desktop/src/types/conversation';
import { useTranslation } from '../../desktop/src/i18n';
import type { PhoneConnection } from './MobileApp';
const status = (webSearch: WebSearchConfig) => invoke<WebSearchProviderStatus[]>('mobile_web_settings', { action: 'status', webSearch });
export function PhoneWebSettings({ connections = [] }: { connections?: PhoneConnection[] }) {
  const { t } = useTranslation();
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => { let active = true; void invoke<AppConfig>('mobile_web_settings', { action: 'read' }).then(value => { if (active) setConfig(value); }).catch(error => { if (active) setError(String(error)); }); return () => { active = false; }; }, []);
  async function save() {
    if (!config || busy) return;
    const submitted = config;
    setBusy(true); setError('');
    try {
      const saved = await invoke<AppConfig>('mobile_web_settings', { action: 'save', webSearch: submitted.webSearch });
      setConfig(current => current === submitted ? saved : current);
    }
    catch (error) { setError(String(error)); }
    finally { setBusy(false); }
  }
  const deepseek = connections.filter(connection => { try { const url = new URL(connection.baseUrl ?? ''); return url.protocol === 'https:' && url.hostname === 'api.deepseek.com' && (!url.port || url.port === '443') && !url.username && !url.password; } catch { return false; } });
  async function useConnection(connectionId: string) {
    if (!config || busy) return;
    const submitted = config;
    setBusy(true); setError('');
    try {
      const saved = await invoke<AppConfig>('mobile_web_settings', { action: 'use_deepseek', connectionId, webSearch: submitted.webSearch });
      setConfig(current => current === submitted ? saved : current);
    } catch (error) { setError(String(error)); } finally { setBusy(false); }
  }
  return <section className="mobile-settings-section mobile-web-settings">{error && <p role="alert" className="mobile-error">{error}</p>}
    {!!deepseek.length && <div className="mobile-settings-form"><p className="mobile-section-hint">{t('remote.mobileDeepseekConnectionHint')}</p>{deepseek.map(connection => <button key={connection.id} disabled={busy} onClick={() => void useConnection(connection.id)}>{t('remote.mobileDeepseekConnection', { name: connection.name })}</button>)}</div>}
    {config && <WebSearchSettingsPanel mobile appConfig={config} loading={busy} onChange={setConfig} onMarkDirty={() => {}} onSave={() => void save()} getStatus={status}/>}</section>;
}
