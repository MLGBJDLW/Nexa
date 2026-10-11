import { useCallback, useEffect, useRef, useState } from 'react';
import { Gauge, Loader2, RefreshCw } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { getCodexAccountSnapshot, getCopilotAccountSnapshot } from '../../lib/api';
import { getExternalAgentUsage } from '../../lib/externalAgents';
import { codexAccountQuota, copilotAccountQuota, externalAccountQuota, type AccountQuota } from '../../lib/subscriptionUsage';
import { SubscriptionUsageDetails } from '../settings/SubscriptionUsageDetails';
import { NexaPopover, NexaPopoverContent, NexaPopoverTrigger } from '../ui/overlay';
import type { AgentConfig } from '../../types/conversation';

const REFRESH_MS = 60_000;

// Mounted with a config/model key: late results cannot migrate to another profile.
export function SubscriptionUsagePopover({ config }: { config: AgentConfig }) {
  const { t, locale } = useTranslation();
  const [open, setOpen] = useState(false);
  const [quota, setQuota] = useState<AccountQuota | null>(null);
  const [loading, setLoading] = useState(false);
  const [updatedAt, setUpdatedAt] = useState<number | null>(null);
  const pending = useRef(false);
  const mounted = useRef(false);
  const refresh = useCallback(async () => {
    if (pending.current) return;
    pending.current = true;
    setLoading(true);
    // Hide the previous read while checking a potentially switched CLI account.
    setQuota(null);
    setUpdatedAt(null);
    try {
      const next = config.provider === 'openai_codex' ? codexAccountQuota(await getCodexAccountSnapshot())
        : config.provider === 'github_copilot' ? copilotAccountQuota(await getCopilotAccountSnapshot())
          : config.provider === 'hermes' ? externalAccountQuota(await getExternalAgentUsage(config.id, config.model))
            : { status: 'unsupported' as const, windows: [] };
      if (mounted.current) { setQuota(next); setUpdatedAt(Date.now()); }
    } catch {
      if (mounted.current) setQuota({ status: 'unavailable', windows: [] });
    } finally {
      pending.current = false;
      if (mounted.current) setLoading(false);
    }
  }, [config.id, config.model, config.provider]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    if (!open) return;
    void refresh();
  }, [open, refresh]);
  useEffect(() => {
    if (!open) return;
    const resume = () => {
      if (document.visibilityState === 'visible' && (!updatedAt || Date.now() - updatedAt >= REFRESH_MS)) void refresh();
    };
    document.addEventListener('visibilitychange', resume);
    return () => document.removeEventListener('visibilitychange', resume);
  }, [open, refresh, updatedAt]);
  useEffect(() => {
    if (!open || loading || !updatedAt || !['openai_codex', 'github_copilot', 'hermes'].includes(config.provider)) return;
    const timer = window.setTimeout(() => { if (document.visibilityState === 'visible') void refresh(); }, REFRESH_MS);
    return () => window.clearTimeout(timer);
  }, [open, loading, updatedAt, config.provider, refresh]);
  return <NexaPopover open={open} onOpenChange={setOpen}>
    <NexaPopoverTrigger asChild>
      <button type="button" data-testid="subscription-usage-trigger" aria-label={t('settings.subscriptionUsage')} title={t('settings.subscriptionUsage')}
        className="flex h-7 w-7 shrink-0 items-center justify-center border-l border-border/60 text-text-secondary transition-colors hover:bg-surface-2 hover:text-accent focus-visible:outline-2 focus-visible:outline-accent">
        <Gauge size={14} aria-hidden="true" />
      </button>
    </NexaPopoverTrigger>
    <NexaPopoverContent side="top" align="end" aria-label={t('settings.subscriptionUsage')}
      className="z-50 max-h-[min(480px,75dvh)] w-[min(320px,calc(100vw-24px))] overflow-y-auto rounded-xl border border-border bg-surface-0 p-3 text-text-primary shadow-xl" data-testid="subscription-usage-popover">
      <div className="mb-3 flex items-center justify-between gap-2">
        <div className="min-w-0"><p className="text-xs font-semibold">{t('settings.subscriptionUsage')}</p><p className="mt-0.5 truncate text-[10px] text-text-tertiary">{config.name}</p></div>
        <button type="button" onClick={() => void refresh()} disabled={loading} aria-label={t('settings.subscriptionRefresh')}
          className="rounded-md p-1.5 text-text-tertiary hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-50">
          {loading ? <Loader2 size={13} className="animate-spin" /> : <RefreshCw size={13} />}
        </button>
      </div>
      {loading ? <p role="status" className="text-xs text-text-tertiary">{t('common.loading')}</p> : quota && <SubscriptionUsageDetails quota={quota} />}
      <p className="mt-3 text-[10px] leading-4 text-text-tertiary">{t('settings.subscriptionQuotaScope')}</p>
      {updatedAt && quota?.status === 'ready' && <p className="mt-1 text-[10px] text-text-tertiary">{t('settings.subscriptionQuotaUpdated', { time: new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit' }).format(updatedAt) })}</p>}
    </NexaPopoverContent>
  </NexaPopover>;
}
