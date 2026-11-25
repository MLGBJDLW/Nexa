import { useLayoutEffect, useMemo, useRef, useState } from 'react';

import type { UsageTimeSeriesPoint } from '../../lib/api';

type UsageHeatmapMode = 'tokens' | 'requests';

interface UsageContributionHeatmapProps {
  points: UsageTimeSeriesPoint[];
  mode: UsageHeatmapMode;
  locale: string;
  valueLabel: string;
  startAt?: string | null;
  endAt?: string | null;
}

interface HeatmapDay {
  key: string;
  date: Date;
  inRange: boolean;
  value: number;
}

interface HeatmapWeek {
  key: string;
  days: HeatmapDay[];
}

const DAY_MS = 24 * 60 * 60 * 1000;
const MAX_VISIBLE_DAYS = 371;
const LEVEL_CLASSES = [
  'bg-surface-3/70',
  'bg-accent/15',
  'bg-accent/30',
  'bg-accent/45',
  'bg-accent/60',
  'bg-accent/80',
  'bg-accent',
] as const;

export function UsageContributionHeatmap({
  points,
  mode,
  locale,
  valueLabel,
  startAt,
  endAt,
}: UsageContributionHeatmapProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const followLatest = useRef(true);
  const [selectedDate, setSelectedDate] = useState<string>();
  const projection = useMemo(
    () => buildHeatmapProjection(points, mode, startAt, endAt),
    [endAt, mode, points, startAt],
  );
  const monthFormatter = useMemo(
    () => new Intl.DateTimeFormat(locale, { month: 'short', timeZone: 'UTC' }),
    [locale],
  );
  const dateFormatter = useMemo(
    () => new Intl.DateTimeFormat(locale, {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      timeZone: 'UTC',
    }),
    [locale],
  );
  const compact = useMemo(
    () => new Intl.NumberFormat(locale, {
      maximumFractionDigits: 1,
      notation: 'compact',
    }),
    [locale],
  );
  const exact = useMemo(() => new Intl.NumberFormat(locale), [locale]);
  const selected = selectedDate ? projection.daysByDate.get(selectedDate) : undefined;
  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    const revealLatest = () => {
      if (followLatest.current) element.scrollLeft = element.scrollWidth - element.clientWidth;
    };
    revealLatest();
    const observer = new ResizeObserver(revealLatest);
    observer.observe(element);
    return () => observer.disconnect();
  }, [projection.weeks.length]);

  if (projection.weeks.length === 0) return null;

  return (
    <div className="mt-4 min-w-0" data-testid="usage-contribution-heatmap" role="group" aria-label={valueLabel}>
      <div
        ref={scrollRef}
        onScroll={(event) => {
          const element = event.currentTarget;
          followLatest.current = element.scrollWidth - element.clientWidth - element.scrollLeft < 2;
        }}
        className="overflow-x-auto pb-1"
      >
        <div className="min-w-max">
          <div
            className="mb-1 grid gap-1 text-[10px] text-text-tertiary"
            style={{ gridTemplateColumns: `repeat(${projection.weeks.length}, 0.75rem)` }}
            aria-hidden="true"
          >
            {projection.weeks.map((week, index) => {
              const firstInRangeDay = week.days.find((day) => day.inRange);
              const currentMonth = firstInRangeDay?.date.getUTCMonth();
              const previousMonth = index > 0
                ? projection.weeks[index - 1].days.find((day) => day.inRange)?.date.getUTCMonth()
                : undefined;
              const showMonth = firstInRangeDay != null && currentMonth !== previousMonth;
              return (
                <span key={`month-${week.key}`} className="h-3 whitespace-nowrap">
                  {showMonth ? monthFormatter.format(firstInRangeDay.date) : ''}
                </span>
              );
            })}
          </div>

          <div
            className="grid grid-flow-col grid-rows-7 gap-1"
            role="grid"
            aria-rowcount={7}
            aria-colcount={projection.weeks.length}
          >
            {projection.weeks.flatMap((week) => week.days.map((day) => {
              const level = projection.bands.find(band => day.value <= band.max)?.level ?? 0;
              const label = `${dateFormatter.format(day.date)} · ${exact.format(day.value)} ${valueLabel}`;
              return (
                <span
                  key={day.key}
                  role="gridcell"
                  aria-label={day.inRange ? label : undefined}
                  title={day.inRange ? label : undefined}
                  onFocus={day.inRange ? () => setSelectedDate(day.key) : undefined}
                  onClick={day.inRange ? () => setSelectedDate(day.key) : undefined}
                  className={`h-3 w-3 rounded-[3px] ring-1 ring-inset ring-border/25 transition-transform hover:scale-125 focus-visible:scale-125 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                    day.inRange ? LEVEL_CLASSES[level] : 'pointer-events-none opacity-0'
                  }`}
                  tabIndex={day.inRange ? 0 : -1}
                />
              );
            }))}
          </div>

        </div>
      </div>
      <div className="mt-2 flex min-h-4 flex-wrap items-center justify-between gap-2 text-[11px] text-text-secondary">
        <span>{selected && `${dateFormatter.format(selected.date)} · ${exact.format(selected.value)} ${valueLabel}`}</span>
        <div className="flex items-center gap-1">
          <span>0</span>
          {projection.bands.map(({ min, max, level }) => (
            <span key={level} title={`${exact.format(min)}–${exact.format(max)} ${valueLabel}`}
              className={`h-2.5 w-2.5 rounded-[2px] ring-1 ring-inset ring-border/25 ${LEVEL_CLASSES[level]}`} />
          ))}
          {projection.peak > 0 && <span>{compact.format(projection.peak)}</span>}
        </div>
      </div>
    </div>
  );
}

function buildHeatmapProjection(
  points: UsageTimeSeriesPoint[],
  mode: UsageHeatmapMode,
  startAt?: string | null,
  endAt?: string | null,
) {
  const valueByDate = new Map<string, number>();
  for (const point of points) {
    const key = point.date.slice(0, 10);
    const value = mode === 'tokens'
      ? point.totalTokens ?? point.promptTokens + point.completionTokens + point.thinkingTokens
      : point.requestCount;
    valueByDate.set(key, (valueByDate.get(key) ?? 0) + value);
  }

  const today = utcDay(new Date());
  let rangeEnd = endAt ? utcDay(new Date(endAt)) : today;
  if (!Number.isFinite(rangeEnd.getTime())) rangeEnd = today;
  if (rangeEnd.getTime() > today.getTime()) rangeEnd = today;

  let rangeStart = startAt ? utcDay(new Date(startAt)) : addUtcDays(rangeEnd, -(MAX_VISIBLE_DAYS - 1));
  if (!Number.isFinite(rangeStart.getTime())) {
    rangeStart = addUtcDays(rangeEnd, -(MAX_VISIBLE_DAYS - 1));
  }
  if (rangeStart.getTime() > rangeEnd.getTime()) rangeStart = rangeEnd;
  if (daysBetween(rangeStart, rangeEnd) + 1 > MAX_VISIBLE_DAYS) {
    rangeStart = addUtcDays(rangeEnd, -(MAX_VISIBLE_DAYS - 1));
  }

  const gridStart = addUtcDays(rangeStart, -rangeStart.getUTCDay());
  const gridEnd = addUtcDays(rangeEnd, 6 - rangeEnd.getUTCDay());
  const weeks: HeatmapWeek[] = [];
  const totals = new Set<number>();
  const daysByDate = new Map<string, HeatmapDay>();

  for (let cursor = gridStart; cursor.getTime() <= gridEnd.getTime(); cursor = addUtcDays(cursor, 7)) {
    const days: HeatmapDay[] = [];
    for (let offset = 0; offset < 7; offset += 1) {
      const date = addUtcDays(cursor, offset);
      const key = dateKey(date);
      const inRange = date.getTime() >= rangeStart.getTime() && date.getTime() <= rangeEnd.getTime();
      const value = inRange ? (valueByDate.get(key) ?? 0) : 0;
      if (value > 0) totals.add(value);
      const day = { key, date, inRange, value };
      days.push(day);
      if (inRange) daysByDate.set(key, day);
    }
    weeks.push({ key: dateKey(cursor), days });
  }

  const sorted = [...totals].sort((a, b) => a - b);
  return { weeks, daysByDate, bands: activityBands(sorted), peak: sorted[sorted.length - 1] ?? 0 };
}

function activityBands(sorted: number[]) {
  const shades = LEVEL_CLASSES.length - 1;
  const bands = [{ min: 0, max: 0, level: 0 }];
  if (sorted.length === 0) return bands;

  // Split the widest occupied interval on a square-root scale. A lone peak
  // takes one shade, and a long tail of quiet days cannot crowd out busy days.
  const ranges = [{ first: 0, last: sorted.length - 1 }];
  const span = ({ first, last }: typeof ranges[number]) => Math.sqrt(sorted[last]) - Math.sqrt(sorted[first]);
  while (ranges.length < Math.min(shades, sorted.length)) {
    const widest = ranges.filter(range => range.first < range.last)
      .reduce((best, range) => span(range) > span(best) ? range : best);
    const midpoint = Math.sqrt(sorted[widest.first]) + span(widest) / 2;
    let split = widest.first + 1;
    while (split < widest.last && Math.sqrt(sorted[split]) < midpoint) split++;
    ranges.splice(ranges.indexOf(widest), 1,
      { first: widest.first, last: split - 1 },
      { first: split, last: widest.last },
    );
  }
  for (const [index, { first, last }] of ranges.entries()) {
    bands.push({
      min: sorted[first],
      max: sorted[last],
      level: ranges.length === 1 ? shades : 1 + Math.round(index * (shades - 1) / (ranges.length - 1)),
    });
  }
  return bands;
}

function utcDay(date: Date): Date {
  return new Date(Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate()));
}

function addUtcDays(date: Date, days: number): Date {
  return new Date(date.getTime() + days * DAY_MS);
}

function daysBetween(start: Date, end: Date): number {
  return Math.floor((end.getTime() - start.getTime()) / DAY_MS);
}

function dateKey(date: Date): string {
  return date.toISOString().slice(0, 10);
}
