import { useTranslation } from '../../i18n';
import type { AccountQuota } from '../../lib/subscriptionUsage';

export function SubscriptionUsageDetails({ quota }: { quota: AccountQuota }) {
  const { t, locale } = useTranslation();
  const percent = (value: number) => new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(value);
  const resetTime = (value: string | null) => {
    const date = value ? new Date(value) : null;
    return date && Number.isFinite(date.getTime())
      ? new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(date) : null;
  };
  if (quota.status !== 'ready') return <p className="text-xs leading-5 text-text-tertiary" role="status">
    {t(quota.status === 'unsupported' ? 'settings.subscriptionQuotaUnsupported'
      : quota.status === 'signedOut' ? 'settings.subscriptionQuotaSignIn' : 'settings.subscriptionQuotaUnavailable')}
  </p>;
  return <div className="space-y-3" data-testid="subscription-usage-details">
    {(quota.plan || quota.provider) && <p className="break-words text-xs font-medium text-text-secondary">{[quota.provider, quota.plan].filter(Boolean).join(' · ')}</p>}
    {quota.windows.map(window => {
      const remaining = window.usedPercent === null ? null : 100 - window.usedPercent;
      const reset = resetTime(window.resetsAt);
      const minutes = window.durationMinutes;
      const duration = typeof minutes === 'number' && minutes > 0 && Number.isFinite(minutes)
        ? new Intl.NumberFormat(locale, { style: 'unit', unit: minutes % 1440 === 0 ? 'day' : minutes % 60 === 0 ? 'hour' : 'minute', unitDisplay: 'short' })
          .format(minutes % 1440 === 0 ? minutes / 1440 : minutes % 60 === 0 ? minutes / 60 : minutes)
        : window.kind ? t(window.kind === 'primary' ? 'settings.subscriptionPrimaryWindow' : 'settings.subscriptionSecondaryWindow') : null;
      const label = [window.label, duration].filter(Boolean).join(' · ');
      return <div key={window.id} className="space-y-1.5 rounded-lg border border-border/60 bg-surface-1/50 p-2.5">
        <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1 text-[11px]">
          <span className="min-w-0 break-words text-text-secondary">{label}</span>
          <span className="font-medium tabular-nums text-text-primary">{window.unlimited ? t('settings.copilotUnlimited')
            : remaining === null ? t('settings.subscriptionQuotaUnknown') : t('settings.subscriptionRemaining', { percent: percent(remaining) })}</span>
        </div>
        {!window.unlimited && remaining !== null && <>
          <div className="h-1 overflow-hidden rounded-full bg-surface-3" role="meter" aria-label={label}
            aria-valuemin={0} aria-valuemax={100} aria-valuenow={remaining} aria-valuetext={t('settings.subscriptionRemaining', { percent: percent(remaining) })}>
            <div className={`h-full rounded-full ${remaining <= 10 ? 'bg-danger' : 'bg-accent'}`} style={{ width: `${remaining}%` }} />
          </div>
          <p className="text-[10px] tabular-nums text-text-tertiary">{t('settings.subscriptionUsed', { percent: percent(window.usedPercent!) })}</p>
        </>}
        {reset && <p className="text-[10px] text-text-tertiary">{t('settings.subscriptionResets', { date: reset })}</p>}
      </div>;
    })}
    {quota.details?.map((detail, index) => <p key={index} className="break-words text-[11px] leading-5 text-text-secondary">{detail}</p>)}
  </div>;
}
