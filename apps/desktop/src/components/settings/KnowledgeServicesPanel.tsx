import { useEffect, useState } from 'react';
import { useTranslation } from '../../i18n';
import * as api from '../../lib/api';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';
import { Section } from './SettingsSection';
import { Search } from 'lucide-react';

export function KnowledgeServicesPanel() {
  const { t } = useTranslation();
  const [config, setConfig] = useState<api.KnowledgeServicesConfig | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let active = true;
    void api.getKnowledgeServicesConfig().then(value => { if (active) { setConfig(value); setError(''); } }).catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; };
  }, [attempt]);
  return <Section icon={<Search size={16} />} title={t('knowledge.servicesTitle')} description={t('knowledge.servicesDescription')} collapsible>
    {error && <p role="alert" className="mb-3 text-xs text-danger">{error} {!config && <button onClick={() => setAttempt(value => value + 1)}>{t('common.retry')}</button>}</p>}
    {config && <div className="space-y-3">
      <label className="block text-xs text-text-secondary">{t('knowledge.rerankerUrl')}<Input aria-label={t('knowledge.rerankerUrl')} value={config.rerankerUrl} placeholder="http://127.0.0.1:8080/rerank" disabled={busy} onChange={event => { setConfig({ ...config, rerankerUrl: event.target.value }); setSaved(false); }} /></label>
      <label className="block text-xs text-text-secondary">{t('knowledge.parserUrl')}<Input aria-label={t('knowledge.parserUrl')} value={config.parserUrl} placeholder="http://127.0.0.1:8091/parse" disabled={busy} onChange={event => { setConfig({ ...config, parserUrl: event.target.value }); setSaved(false); }} /></label>
      <p className="text-xs text-text-tertiary">{t('knowledge.servicesBudget', { candidates: config.maxCandidates, seconds: config.rerankerTimeoutMs / 1000 })}</p>
      <div className="flex items-center gap-3"><Button size="sm" loading={busy} onClick={() => {
        setBusy(true); setError(''); setSaved(false);
        void api.saveKnowledgeServicesConfig(config).then(() => setSaved(true)).catch(cause => setError(String(cause))).finally(() => setBusy(false));
      }}>{t('common.save')}</Button>{saved && <span role="status" className="text-xs text-success">{t('common.success')}</span>}</div>
    </div>}
  </Section>;
}
