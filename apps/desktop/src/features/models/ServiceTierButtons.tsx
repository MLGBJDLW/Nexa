import { Zap } from 'lucide-react';
import { useTranslation } from '../../i18n';
import type { ServiceTier } from './modelChoices';

export function ServiceTierButtons({ tiers, value, onChange, disabled }: {
  tiers: ServiceTier[] | undefined;
  value: ServiceTier | null | undefined;
  disabled?: boolean;
  onChange: (tier: ServiceTier | null) => void;
}) {
  const { t } = useTranslation();
  const available = (['fast', 'ultrafast'] as const).filter(tier => tiers?.includes(tier));
  if (!available.length) return null;
  return <div role="group" aria-label={t('chat.serviceTier')} className="mb-2 flex flex-wrap items-center gap-1.5" data-testid="service-tiers">
    {available.map(tier => {
      const active = tier === value;
      return <button key={tier} type="button" aria-pressed={active} disabled={disabled}
        className={`inline-flex min-h-8 items-center gap-1 rounded-md border px-2 text-xs font-medium transition-colors focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-60 ${active ? 'border-accent/50 bg-accent-subtle text-accent' : 'border-border/70 bg-surface-1 text-text-secondary hover:bg-surface-2'}`}
        onClick={() => onChange(active ? null : tier)}>
        <Zap size={13} aria-hidden="true" fill={active ? 'currentColor' : 'none'} />
        {t(tier === 'fast' ? 'chat.serviceTierFast' : 'chat.serviceTierUltrafast')}
      </button>;
    })}
    <span className="text-[10px] leading-tight text-text-tertiary">{t('chat.serviceTierUsage')}</span>
  </div>;
}
