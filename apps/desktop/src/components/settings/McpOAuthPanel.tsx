import { useEffect, useRef, useState } from 'react';
import { useTranslation } from '../../i18n';
import type { TranslationKey } from '../../i18n';
import * as api from '../../lib/api';
import type { McpOAuthStatus } from '../../types/extensions';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';

export function McpOAuthDisclosure({ serverId, enabled }: { serverId: string; enabled: boolean }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return <details className="border-t border-border/50 px-4 py-2" onToggle={(event) => setOpen(event.currentTarget.open)}>
    <summary className="cursor-pointer text-xs text-text-secondary">{t('settings.mcpOAuth')}</summary>
    {open && <McpOAuthPanel key={serverId} serverId={serverId} enabled={enabled} />}
  </details>;
}

function McpOAuthPanel({ serverId, enabled }: { serverId: string; enabled: boolean }) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<McpOAuthStatus | null>(null);
  const [clientId, setClientId] = useState('');
  const [scopes, setScopes] = useState('');
  const [issuer, setIssuer] = useState('');
  const [resource, setResource] = useState('');
  const [port, setPort] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [receipt, setReceipt] = useState('');
  const sequence = useRef(0);
  const actionSequence = useRef(0);
  useEffect(() => {
    const request = ++sequence.current;
    void api.getMcpOAuthStatus(serverId).then((value) => {
      if (sequence.current !== request) return;
      setStatus(value); setClientId(value.config?.clientId ?? ''); setScopes(value.config?.scopes.join(' ') ?? '');
      setIssuer(value.config?.issuer ?? ''); setResource(value.config?.resource ?? ''); setPort(String(value.config?.redirectPort ?? ''));
    }).catch((error: unknown) => { if (sequence.current === request) setError(String(error)); });
    return () => { sequence.current++; actionSequence.current++; };
  }, [serverId, enabled]);
  useEffect(() => {
    if (status?.status !== 'authorizing' || busy) return;
    let cancelled = false;
    let timer: number;
    const poll = async () => {
      try { const value = await api.getMcpOAuthStatus(serverId); if (!cancelled) setStatus(value); }
      catch (error: unknown) { if (!cancelled) setError(String(error)); }
      if (!cancelled) timer = window.setTimeout(() => void poll(), 1000);
    };
    timer = window.setTimeout(() => void poll(), 1000);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [serverId, status?.status, busy]);
  const config = { clientId: clientId.trim() || null, scopes: scopes.split(/\s+/).filter(Boolean), issuer: issuer.trim() || null, resource: resource.trim() || null, redirectPort: port ? Number(port) : null };
  const saved = status?.config;
  const dirty = !saved || config.clientId !== saved.clientId || config.issuer !== saved.issuer || config.resource !== saved.resource || config.redirectPort !== saved.redirectPort || config.scopes.join(' ') !== saved.scopes.join(' ');
  const invalidPort = port !== '' && (!Number.isInteger(Number(port)) || Number(port) < 1 || Number(port) > 65535);
  const run = async (action: () => Promise<{ status: McpOAuthStatus; receipt?: string }>, authorizing = false) => {
    const request = ++actionSequence.current;
    setBusy(true); setError(''); setReceipt('');
    if (authorizing) setStatus((previous) => previous ? { ...previous, status: 'authorizing' } : previous);
    try {
      const value = await action();
      if (request === actionSequence.current) { setStatus(value.status); setReceipt(value.receipt ?? ''); }
    } catch (error: unknown) {
      if (request === actionSequence.current) { setError(String(error)); setStatus((previous) => previous?.status === 'authorizing' ? { ...previous, status: 'login_failed' } : previous); }
    } finally { if (request === actionSequence.current) setBusy(false); }
  };
  const statusKey = ({ not_configured: 'settings.mcpOAuthNotConfigured', signed_out: 'settings.mcpOAuthSignedOut', authorizing: 'settings.mcpOAuthAuthorizing', connected: 'settings.mcpOAuthConnected', disconnected: 'settings.mcpOAuthDisconnected', refresh_failed: 'settings.mcpOAuthRefreshFailed', reauthorization_required: 'settings.mcpOAuthNeedsLogin', login_failed: 'settings.mcpOAuthLoginFailed' } as Record<string, TranslationKey>)[status?.status ?? ''] ?? 'common.loading';
  return <div className="mt-3 space-y-3" aria-label={t('settings.mcpOAuth')}>
    <p className="text-xs leading-5 text-text-tertiary">{t('settings.mcpOAuthHint')}</p>
    <p role="status" className="text-sm text-text-primary">{t(statusKey)}</p>
    {status?.expiresAt && <p className="text-xs text-text-tertiary">{t('settings.mcpOAuthExpires')}: {new Date(status.expiresAt * 1000).toLocaleString()}</p>}
    {status?.scopes.length ? <p className="break-words text-xs text-text-secondary">{t('settings.mcpOAuthScopes')}: {status.scopes.join(' ')}</p> : null}
    <fieldset disabled={busy || status?.status === 'authorizing' || !status} className="space-y-3 disabled:opacity-70">
      <label className="block space-y-1 text-xs text-text-secondary"><span>{t('settings.mcpOAuthClientId')}</span><Input value={clientId} onChange={(e) => setClientId(e.target.value)} /></label>
      <label className="block space-y-1 text-xs text-text-secondary"><span>{t('settings.mcpOAuthScopes')}</span><Input value={scopes} onChange={(e) => setScopes(e.target.value)} /></label>
      <details><summary className="cursor-pointer text-xs text-text-tertiary">{t('settings.advancedSettings')}</summary><div className="mt-2 space-y-2">
        <label className="block space-y-1 text-xs text-text-secondary"><span>{t('settings.mcpOAuthIssuer')}</span><Input value={issuer} onChange={(e) => setIssuer(e.target.value)} /></label>
        <label className="block space-y-1 text-xs text-text-secondary"><span>{t('settings.mcpOAuthResource')}</span><Input value={resource} onChange={(e) => setResource(e.target.value)} /></label>
        <label className="block space-y-1 text-xs text-text-secondary"><span>{t('settings.mcpOAuthPort')}</span><Input type="number" min={1} max={65535} value={port} onChange={(e) => setPort(e.target.value)} /></label>
      </div></details>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" disabled={!dirty || invalidPort} onClick={() => void run(async () => ({ status: await api.configureMcpOAuth(serverId, config) }))}>{t('settings.mcpOAuthSave')}</Button>
        <Button size="sm" variant="ghost" disabled={!status?.config || dirty || !enabled} onClick={() => void run(async () => ({ status: await api.beginMcpOAuth(serverId) }), true)}>{t('settings.mcpOAuthSignIn')}</Button>
        {status?.config && <Button size="sm" variant="ghost" onClick={() => void run(async () => ({ status: await api.configureMcpOAuth(serverId, null) }))}>{t('settings.mcpOAuthRemove')}</Button>}
      </div>
    </fieldset>
    {status?.config && <Button size="sm" variant="ghost" disabled={busy && status.status !== 'authorizing'} onClick={() => void run(async () => {
      const result = await api.disconnectMcpOAuth(serverId, status.status !== 'authorizing');
      return { status: await api.getMcpOAuthStatus(serverId), receipt: t(result.remoteRevocation === 'confirmed' ? 'settings.mcpOAuthRevoked' : result.remoteRevocation === 'failed' ? 'settings.mcpOAuthRevokeFailed' : 'settings.mcpOAuthLocalOnly') };
    })}>{t(status.status === 'authorizing' ? 'common.cancel' : 'settings.mcpOAuthDisconnect')}</Button>}
    {!enabled && <p className="text-xs text-text-tertiary">{t('settings.mcpOAuthEnable')}</p>}
    {(error || status?.detail) && <p role="alert" className="break-words text-xs text-danger">{error || status?.detail}</p>}
    {receipt && <p className="text-xs text-text-secondary">{receipt}</p>}
  </div>;
}
