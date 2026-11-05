import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { WebSearchSettingsPanel } from '../../desktop/src/components/settings/WebSearchSettingsPanel';
import type { AppConfig, WebSearchConfig, WebSearchProviderStatus } from '../../desktop/src/types/conversation';
const status = (webSearch: WebSearchConfig) => invoke<WebSearchProviderStatus[]>('mobile_web_settings', { action: 'status', webSearch });
export function PhoneWebSettings() {
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
  return <section className="mobile-settings-section">{error && <p role="alert" className="mobile-error">{error}</p>}{config && <WebSearchSettingsPanel appConfig={config} loading={busy} onChange={setConfig} onMarkDirty={() => {}} onSave={() => void save()} getStatus={status}/>}</section>;
}
