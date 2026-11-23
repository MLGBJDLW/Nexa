import { useEffect, useState } from 'react';
import { useTranslation } from '../../i18n';
import * as api from '../../lib/api';
import { loadProviderModelCatalog } from '../../lib/providerModelCatalog';
import { providerCapabilityLinks, type CapabilityPatch, type CapabilityRoute } from '../../lib/providerCapabilityLinks';
import type { AgentConfig } from '../../types/conversation';
import { Button } from '../ui/Button';
import { Modal } from '../ui/Modal';
import { CapabilityLinkDialog } from './CapabilityLinkDialog';

interface Props {
  source: AgentConfig;
  onDismiss: () => void;
  onLinked?: (patch: CapabilityPatch) => void;
}

export function ProviderCapabilityPrompt({ source, onDismiss, onLinked }: Props) {
  const { t } = useTranslation();
  const [routes, setRoutes] = useState<CapabilityRoute[] | null>(null);
  const [error, setError] = useState('');
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let active = true;
    setError('');
    Promise.all([api.getAppConfig(), api.getEmbedderConfig()]).then(([app, embed]) => {
      if (active) setRoutes(providerCapabilityLinks(source, app, embed, t, loadProviderModelCatalog(source.provider, source.baseUrl, source.apiKey)));
    }).catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; };
  }, [source, attempt, t]);
  useEffect(() => { if (routes?.length === 0) onDismiss(); }, [routes, onDismiss]);

  if (routes === null) return <Modal open title={t('providers.capabilityTitle', { name: source.name })} onClose={onDismiss}
    footer={<><Button variant="secondary" onClick={onDismiss}>{t('providers.capabilitySkip')}</Button>
      {error && <Button onClick={() => setAttempt(n => n + 1)}>{t('common.retry')}</Button>}</>}>
    {error ? <p role="alert">{error}</p> : <p role="status">{t('common.loading')}</p>}
  </Modal>;
  if (!routes.length) return null;

  return <CapabilityLinkDialog providerName={source.name} options={routes} onDismiss={onDismiss} onApply={async ids => {
    const patch: CapabilityPatch = {};
    for (const id of ids) {
      const route = routes.find(row => row.id === id && row.patch && !row.disabled && !row.linked);
      if (!route) throw new Error(t('common.error'));
      Object.assign(patch, route.patch);
    }
    await api.linkProviderCapabilities(source.id, patch);
    onLinked?.(patch);
  }} />;
}
