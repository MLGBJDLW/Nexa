import { Clock3 } from 'lucide-react';
import { useTranslation } from '../../i18n';
import type { EmbeddingEstimate } from '../../types/ingest';

export function EmbeddingTiming({ estimate }: { estimate?: EmbeddingEstimate }) {
  const { t, locale } = useTranslation();
  if (!estimate) return null;
  const duration = (seconds: number) => {
    const unit = seconds >= 3600 ? 'hour' : seconds >= 60 ? 'minute' : 'second';
    const value = seconds / (unit === 'hour' ? 3600 : unit === 'minute' ? 60 : 1);
    return new Intl.NumberFormat(locale, { style: 'unit', unit, unitDisplay: 'short', maximumFractionDigits: value < 10 ? 1 : 0 }).format(value);
  };
  const remaining = estimate.estimatedRemainingSeconds;
  return <div data-testid="embedding-timing" className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] leading-5 text-text-secondary">
    <Clock3 size={12} aria-hidden="true" className="shrink-0 text-accent" />
    <span className="max-w-48 truncate font-medium" title={estimate.model}>{estimate.model}</span>
    <span className="tabular-nums">{remaining == null ? t('sources.embeddingCalibrating') : t('sources.embeddingRemaining', { time: duration(remaining) })}</span>
    <span className="text-text-tertiary">{t('sources.embeddingElapsed', { time: duration(estimate.elapsedSeconds) })}</span>
    {estimate.basis !== 'calibrating' && <span className="text-text-tertiary">· {t(estimate.basis === 'history' ? 'sources.embeddingHistory' : 'sources.embeddingMeasured')}</span>}
  </div>;
}
