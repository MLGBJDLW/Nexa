import { useEffect, useRef, type ReactNode } from 'react';
import { useReducedMotion } from 'framer-motion';
import { useTranslation } from '../../i18n';

interface DiffStatsTickerProps {
  additions: number;
  deletions: number;
  filesChanged?: number;
  compact?: boolean;
  live?: boolean;
  showFiles?: boolean;
  testIdPrefix?: string;
}

function normalizedCount(value: number | null | undefined): number {
  return Number.isFinite(value) ? Math.max(0, Math.round(value ?? 0)) : 0;
}

export function RollingCount({ value, className }: { value: number; className?: string }) {
  const shouldReduceMotion = useReducedMotion();
  const count = normalizedCount(value);
  const previousRef = useRef(count);
  const rootRef = useRef<HTMLSpanElement>(null);
  const digits = String(count).split('');

  useEffect(() => {
    const previous = String(previousRef.current);
    previousRef.current = count;
    const root = rootRef.current;
    if (!root || previous === String(count) || shouldReduceMotion || window.matchMedia('(prefers-reduced-motion: reduce)').matches || document.visibilityState !== 'visible') return;
    const bounds = root.getBoundingClientRect();
    if (!bounds.width || !bounds.height || bounds.bottom < 0 || bounds.top > window.innerHeight) return;
    const animations: Animation[] = [];
    root.querySelectorAll<HTMLElement>('[data-count-place]').forEach(place => {
      const index = Number(place.dataset.countPlace);
      const current = String(count);
      const digit = current[current.length - index - 1];
      if (previous[previous.length - index - 1] === digit) return;
      const base = place.querySelector<HTMLElement>('[data-count-digit]');
      const strip = place.querySelector<HTMLElement>('[data-count-scramble]');
      if (!base || !strip) return;
      const alphabet = '7083#129%46';
      strip.dataset.scramble = Array.from({ length: 6 }, (_, frame) => alphabet[(count + index * 7 + frame * 3) % alphabet.length]).concat(digit).join('\n');
      const duration = 260 + Math.min(index, 3) * 18;
      animations.push(base.animate([{ opacity: 0 }, { opacity: 0, offset: 0.92 }, { opacity: 1 }], { duration }));
      animations.push(strip.animate([
        { transform: 'translateY(0)', opacity: 1 },
        { transform: 'translateY(-6em)', opacity: 1, offset: 0.92 },
        { transform: 'translateY(-6em)', opacity: 0 },
      ], { duration, easing: 'steps(6, end)' }));
    });
    return () => animations.forEach(animation => animation.cancel());
  }, [count, shouldReduceMotion]);

  return <span ref={rootRef} data-count-value={count} className={`inline-block text-right tabular-nums leading-none ${className ?? ''}`}>
    {digits.map((digit, index) => <span key={digits.length - index - 1} data-count-place={digits.length - index - 1}
      className="relative inline-block w-[1ch] text-center align-baseline">
      <span data-count-digit>{digit}</span>
      <span aria-hidden="true" className="pointer-events-none absolute inset-0 overflow-hidden">
        <span data-count-scramble data-scramble="" className="absolute inset-x-0 top-0 whitespace-pre leading-none opacity-0 before:content-[attr(data-scramble)]" />
      </span>
    </span>)}
  </span>;
}

function SignedRollingCount({ value, sign }: { value: number; sign: '+' | '-' }) {
  return (
    <span className="inline-flex min-w-[2.6ch] items-baseline justify-end tabular-nums leading-none">
      <span className="shrink-0">{sign}</span>
      <RollingCount value={value} />
    </span>
  );
}

function StatPill({
  tone,
  kind,
  valueLabel,
  testId,
  compact,
  live,
  children,
}: {
  tone: 'add' | 'delete' | 'neutral';
  kind: 'additions' | 'deletions' | 'files';
  valueLabel: string;
  testId?: string;
  compact?: boolean;
  live?: boolean;
  children: ReactNode;
}) {
  const size = compact ? 'h-5 px-1.5 text-[10px]' : 'h-6 px-2 text-[11px]';
  const toneClass =
    tone === 'add'
      ? 'border-success/25 bg-success/10 text-success'
      : tone === 'delete'
        ? 'border-danger/25 bg-danger/10 text-danger'
        : 'border-border/60 bg-surface-0/70 text-text-tertiary';

  return (
    <span
      data-testid={testId}
      data-diff-stat-kind={kind}
      data-value={valueLabel}
      className={`${size} inline-flex items-center gap-1 rounded-md border font-mono leading-none shadow-[inset_0_1px_0_rgba(255,255,255,0.04)] ${toneClass} ${
        live ? 'motion-safe:animate-pulse' : ''
      }`}
    >
      {children}
    </span>
  );
}

export function DiffStatsTicker({
  additions,
  deletions,
  filesChanged = 1,
  compact = false,
  live = false,
  showFiles = true,
  testIdPrefix,
}: DiffStatsTickerProps) {
  const { t } = useTranslation();
  const additionsValue = normalizedCount(additions);
  const deletionsValue = normalizedCount(deletions);
  const filesValue = normalizedCount(filesChanged);
  const statTestId = (kind: string) => (testIdPrefix ? `${testIdPrefix}-${kind}` : undefined);

  return (
    <div
      data-testid={testIdPrefix ? `${testIdPrefix}-diff-stats` : undefined}
      className="inline-flex shrink-0 items-center gap-1 tabular-nums"
      aria-label={`+${additionsValue} -${deletionsValue}`}
      data-live-diff-stats={live ? 'true' : 'false'}
    >
      <StatPill
        tone="add"
        kind="additions"
        valueLabel={`+${additionsValue}`}
        testId={statTestId('additions')}
        compact={compact}
        live={live}
      >
        <SignedRollingCount value={additionsValue} sign="+" />
      </StatPill>
      <StatPill
        tone="delete"
        kind="deletions"
        valueLabel={`-${deletionsValue}`}
        testId={statTestId('deletions')}
        compact={compact}
        live={live}
      >
        <SignedRollingCount value={deletionsValue} sign="-" />
      </StatPill>
      {showFiles && filesValue > 1 && (
        <StatPill
          tone="neutral"
          kind="files"
          valueLabel={String(filesValue)}
          testId={statTestId('files')}
          compact={compact}
        >
          <RollingCount value={filesValue} />
          <span className="font-sans">{t('chat.diffFiles')}</span>
        </StatPill>
      )}
    </div>
  );
}
