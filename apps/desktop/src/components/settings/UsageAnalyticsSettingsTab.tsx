import { save } from '@tauri-apps/plugin-dialog';
import { NexaSelect } from '../ui/overlay';
import { CalendarDays, Download, RefreshCw, Trash2 } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { toast } from 'sonner';

import * as api from '../../lib/api';
import { useUsageAnalytics } from '../../features/usage/useUsageAnalytics';
import { usageModelIdentity } from '../../features/usage/modelIdentity';
import { useTranslation } from '../../i18n';
import { UsageContributionHeatmap } from './UsageContributionHeatmap';
import { UsageModelBreakdown, usageProviderLabel } from './UsageModelBreakdown';

type RangePreset = 'today' | '7d' | '30d' | '90d' | 'year' | 'all' | 'custom';

export function UsageAnalyticsSettingsTab() {
  const { t, locale } = useTranslation();
  const { filter, setFilter, data, loading, error, reload } = useUsageAnalytics(rangeFilter('30d'));
  const {
    filter: activityFilter,
    setFilter: setActivityFilter,
    data: activityData,
    loading: activityLoading,
    error: activityError,
    reload: reloadActivity,
  } = useUsageAnalytics(contributionActivityFilter());
  const [range, setRange] = useState<RangePreset>('30d');
  const [customStart, setCustomStart] = useState('');
  const [customEnd, setCustomEnd] = useState('');
  const [chartMode, setChartMode] = useState<'tokens' | 'requests'>('tokens');
  const [selectedModel, setSelectedModel] = useState('');
  const options = useMemo(() => {
    const snapshots = [data, activityData];
    const unique = (values: (string | null | undefined)[]) => [...new Set(values.filter(Boolean) as string[])].sort();
    return {
      providers: unique([filter.providerId, ...snapshots.flatMap((snapshot) => snapshot?.availableFilters?.providers ?? snapshot?.byModel.map((row) => row.providerId) ?? [])]),
      models: unique([filter.modelId, ...(filter.modelIds ?? []), ...snapshots.flatMap((snapshot) => snapshot?.availableFilters?.models ?? snapshot?.byModel.map((row) => row.modelId) ?? [])]),
      operations: unique([filter.operationKind, ...snapshots.flatMap((snapshot) => snapshot?.availableFilters?.operations ?? snapshot?.byOperation.map((row) => row.key) ?? [])]),
    };
  }, [activityData, data, filter.providerId, filter.modelId, filter.modelIds, filter.operationKind]);
  const modelChoices = useMemo(() => [...new Set(options.models.map(usageModelIdentity))].sort(), [options.models]);

  useEffect(() => {
    if (!selectedModel) return;
    const ids = options.models.filter((id) => usageModelIdentity(id) === selectedModel);
    if (JSON.stringify(ids) !== JSON.stringify(filter.modelIds)) {
      setFilter((previous) => ({ ...previous, modelId: null, modelIds: ids }));
    }
  }, [selectedModel, options.models, filter.modelIds, setFilter]);

  useEffect(() => {
    setActivityFilter({
      ...contributionActivityFilter(),
      providerId: filter.providerId ?? null,
      modelId: filter.modelId ?? null,
      modelIds: filter.modelIds ?? null,
      operationKind: filter.operationKind ?? null,
      timeBucket: 'day',
    });
  }, [filter.modelId, filter.modelIds, filter.operationKind, filter.providerId, setActivityFilter]);

  const reloadAll = () => {
    reload();
    reloadActivity();
  };
  const chooseRange = (preset: RangePreset) => {
    setRange(preset);
    if (preset !== 'custom') setFilter({
      ...rangeFilter(preset),
      providerId: filter.providerId ?? null,
      modelId: filter.modelId ?? null,
      modelIds: filter.modelIds ?? null,
      operationKind: filter.operationKind ?? null,
      timeBucket: filter.timeBucket ?? 'day',
    });
  };
  const applyCustom = () => {
    const end = customEnd ? new Date(`${customEnd}T00:00:00`) : null;
    if (end) end.setDate(end.getDate() + 1); // The ledger uses an exclusive end.
    setFilter({ ...filter,
      startAt: customStart ? new Date(`${customStart}T00:00:00`).toISOString() : null,
      endAt: end?.toISOString() ?? null,
    });
  };
  const exportData = async (format: 'csv' | 'json') => {
    const path = await save({ defaultPath: `nexa-ai-usage.${format}`, filters: [{ name: format.toUpperCase(), extensions: [format] }] });
    if (!path) return;
    try { await api.exportAiUsage(filter, format, path); toast.success(t('usage.exported', { path })); }
    catch (reason) { console.error('[usage] export failed', reason); toast.error(t('usage.exportFailed')); }
  };
  const remove = async (all: boolean) => {
    const targetFilter = all ? {} : filter;
    if (!window.confirm(t(all ? 'usage.deleteAllConfirm' : 'usage.deleteRangeConfirm'))) return;
    try {
      const count = await api.deleteAiUsageRecords(targetFilter);
      toast.success(t('usage.deleted', { count }));
      reloadAll();
    } catch (reason) { console.error('[usage] delete failed', reason); toast.error(t('usage.deleteFailed')); }
  };

  if (loading && !data) return <div className="rounded-xl border border-border bg-surface-1 p-8 text-sm text-text-tertiary">{t('usage.loading')}</div>;
  if (error && !data) return <div className="rounded-xl border border-danger/30 bg-danger/10 p-5 text-sm text-danger">{t('usage.loadFailed')}</div>;
  if (!data) return null;
  const totals = data.totals;

  return (
    <div className="min-w-0 space-y-3" data-testid="usage-analytics-tab" aria-busy={loading || activityLoading}>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-base font-semibold text-text-primary">{t('usage.title')}</h2>
          <p className="mt-1 max-w-2xl text-xs text-text-tertiary">{t('usage.description')}</p>
        </div>
        <button type="button" onClick={reloadAll} className="inline-flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs text-text-secondary hover:bg-surface-2"><RefreshCw size={13} /> {t('usage.refresh')}</button>
      </div>

      <div className="flex flex-wrap items-center gap-2 rounded-lg border border-border bg-surface-1 p-2">
        <CalendarDays size={14} className="ml-1 text-text-tertiary" />
        {(['today', '7d', '30d', '90d', 'year', 'all', 'custom'] as RangePreset[]).map((preset) => (
          <button key={preset} type="button" onClick={() => chooseRange(preset)} className={`rounded-md px-2.5 py-1.5 text-xs ${range === preset ? 'bg-accent text-text-inverse' : 'text-text-secondary hover:bg-surface-2'}`}>{rangeLabel(preset, t)}</button>
        ))}
      </div>
      <div className="grid gap-2 sm:grid-cols-3" data-testid="usage-filters">
        <NexaSelect aria-label={t('usage.allProviders')} value={filter.providerId ?? ''} onChange={(event) => setFilter({ ...filter, providerId: event.target.value || null })} className="w-full min-w-0 rounded-lg border border-border bg-surface-1 px-3 py-2 text-xs text-text-primary"><option value="">{t('usage.allProviders')}</option>{options.providers.map((provider) => <option key={provider} value={provider}>{usageProviderLabel(provider)}</option>)}</NexaSelect>
        <NexaSelect aria-label={t('usage.allModels')} value={selectedModel} onChange={(event) => { const model = event.target.value; setSelectedModel(model); setFilter({ ...filter, modelId: null, modelIds: model ? options.models.filter((id) => usageModelIdentity(id) === model) : null }); }} className="w-full min-w-0 rounded-lg border border-border bg-surface-1 px-3 py-2 text-xs text-text-primary"><option value="">{t('usage.allModels')}</option>{modelChoices.map((model) => <option key={model}>{model}</option>)}</NexaSelect>
        <NexaSelect aria-label={t('usage.allOperations')} value={filter.operationKind ?? ''} onChange={(event) => setFilter({ ...filter, operationKind: event.target.value || null })} className="w-full min-w-0 rounded-lg border border-border bg-surface-1 px-3 py-2 text-xs text-text-primary"><option value="">{t('usage.allOperations')}</option>{options.operations.map((operation) => <option key={operation} value={operation}>{operationLabel(operation, t)}</option>)}</NexaSelect>
      </div>
      {(filter.providerId || selectedModel || filter.operationKind) && <button type="button" className="text-xs text-accent hover:underline" onClick={() => { setSelectedModel(''); setFilter({ ...filter, providerId: null, modelId: null, modelIds: null, operationKind: null }); }}>{t('usage.clearFilters')}</button>}
      {(error || activityError) && <p role="alert" className="text-xs text-danger">{t('usage.loadFailed')}</p>}
      {range === 'custom' && <div className="flex flex-wrap items-end gap-2 rounded-lg border border-border bg-surface-1 p-3"><label className="text-xs text-text-secondary">{t('usage.from')}<input type="date" value={customStart} onChange={(event) => setCustomStart(event.target.value)} className="ml-2 rounded-md border border-border bg-surface-0 px-2 py-1.5 text-text-primary" /></label><label className="text-xs text-text-secondary">{t('usage.to')}<input type="date" value={customEnd} onChange={(event) => setCustomEnd(event.target.value)} className="ml-2 rounded-md border border-border bg-surface-0 px-2 py-1.5 text-text-primary" /></label><button type="button" onClick={applyCustom} className="rounded-md bg-accent px-3 py-1.5 text-xs text-text-inverse">{t('usage.apply')}</button></div>}

      <div className="grid grid-cols-2 gap-3 lg:grid-cols-5">
        <Metric label={t('usage.totalTokens')} value={formatNumber(totals.totalTokens, locale)} />
        <Metric label={t('usage.inputTokens')} value={formatNumber(totals.promptTokens, locale)} />
        <Metric label={t('usage.outputTokens')} value={formatNumber(totals.completionTokens, locale)} />
        <Metric label={t('usage.thinkingTokens')} value={formatNumber(totals.thinkingTokens, locale)} />
        <Metric label={t('usage.requests')} value={formatNumber(totals.requestCount, locale)} />
        <Metric label={t('usage.agentRuns')} value={formatNumber(totals.agentRunCount, locale)} />
        <Metric label={t('usage.cacheRead')} value={formatNumber(totals.cacheReadTokens, locale)} />
        <Metric label={t('usage.cacheMiss')} value={formatNumber(totals.cacheMissTokens, locale)} />
        <Metric label={t('usage.cacheHitRate')} value={totals.cacheHitRate == null ? '—' : `${totals.cacheHitRate.toFixed(1)}%`} />
        <Metric label={t('usage.estimatedCost')} value={formatCost(totals, t)} hint={totals.estimatedCostMicros == null ? t('usage.costUnknownHint') : undefined} />
      </div>

      <section className="rounded-xl border border-border bg-surface-1 p-4">
        <div className="flex items-center justify-between gap-3"><h3 className="text-sm font-semibold text-text-primary">{t('usage.coverage')}</h3><span className="text-[11px] text-text-tertiary">{t('usage.coverageHint')}</span></div>
        <div className="mt-3 flex h-3 overflow-hidden rounded-full bg-surface-3" title={t('usage.coverageTitle')}>
          <span className="bg-success" style={{ width: `${totals.providerReportedPercent}%` }} />
          <span className="bg-info" style={{ width: `${totals.normalizedPercent}%` }} />
          <span className="bg-warning" style={{ width: `${totals.estimatedPercent}%` }} />
          <span className="bg-danger" style={{ width: `${totals.unknownPercent}%` }} />
        </div>
        <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-text-tertiary"><span>{t('usage.providerReported')} {totals.providerReportedPercent.toFixed(1)}%</span><span>{t('usage.normalized')} {totals.normalizedPercent.toFixed(1)}%</span><span>{t('usage.estimated')} {totals.estimatedPercent.toFixed(1)}%</span><span>{t('usage.unknown')} {totals.unknownPercent.toFixed(1)}%</span></div>
      </section>

      <section className="rounded-xl border border-border bg-surface-1 p-4">
        <div className="flex flex-wrap items-center justify-between gap-2"><div><h3 className="text-sm font-semibold text-text-primary">{t('usage.trend')}</h3><span className="text-[11px] text-text-tertiary">{t('usage.activityYear')}</span></div><NexaSelect aria-label={t('usage.trend')} value={chartMode} onChange={(event) => setChartMode(event.target.value as 'tokens' | 'requests')} className="rounded-md border border-border bg-surface-0 px-2 py-1 text-[11px] text-text-primary"><option value="tokens">{t('usage.tokens')}</option><option value="requests">{t('usage.requests')}</option></NexaSelect></div>
        {activityLoading && !activityData ? (
          <div className="mt-4 h-28 animate-pulse rounded-lg bg-surface-2" />
        ) : activityError && !activityData ? (
          <div className="mt-4 rounded-lg border border-danger/30 bg-danger/10 p-3 text-xs text-danger">{t('usage.loadFailed')}</div>
        ) : (
          <UsageContributionHeatmap
            points={activityData?.timeSeries ?? []}
            mode={chartMode}
            locale={locale}
            valueLabel={chartMode === 'tokens' ? t('usage.tokens') : t('usage.requests')}
            startAt={activityFilter.startAt}
            endAt={activityFilter.endAt}
          />
        )}
      </section>

      <UsageModelBreakdown rows={data.byModel} />
      <Breakdown title={t('usage.operation')} rows={data.byOperation} totals={totals} t={t} locale={locale} />

      <div className="flex flex-wrap justify-between gap-2 rounded-xl border border-border bg-surface-1 p-4">
        <div className="flex gap-2"><button type="button" onClick={() => void exportData('csv')} className="inline-flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs text-text-secondary"><Download size={13} /> {t('usage.exportCsv')}</button><button type="button" onClick={() => void exportData('json')} className="inline-flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs text-text-secondary"><Download size={13} /> {t('usage.exportJson')}</button></div>
        <div className="flex gap-2"><button type="button" onClick={() => void remove(false)} className="inline-flex items-center gap-1.5 rounded-md border border-warning/40 px-2.5 py-1.5 text-xs text-warning"><Trash2 size={13} /> {t('usage.deleteRange')}</button><button type="button" onClick={() => void remove(true)} className="inline-flex items-center gap-1.5 rounded-md border border-danger/40 px-2.5 py-1.5 text-xs text-danger"><Trash2 size={13} /> {t('usage.resetAll')}</button></div>
      </div>
    </div>
  );
}

function Metric({ label, value, hint }: { label: string; value: string; hint?: string }) { return <div className="rounded-xl border border-border bg-surface-1 p-3"><div className="text-[11px] text-text-tertiary">{label}</div><div className="mt-1 text-lg font-semibold tabular-nums text-text-primary">{value}</div>{hint && <div className="mt-1 text-[10px] text-text-tertiary">{hint}</div>}</div>; }
type Translate = ReturnType<typeof useTranslation>['t'];
function Empty({ t }: { t: Translate }) { return <div className="mt-4 rounded-lg border border-dashed border-border p-6 text-center text-xs text-text-tertiary">{t('usage.empty')}</div>; }
function Breakdown({ title, rows, totals, t, locale }: { title: string; rows: api.UsageBreakdownRow[]; totals: api.UsageTotals; t: Translate; locale: string }) { return <section className="overflow-hidden rounded-xl border border-border bg-surface-1"><h3 className="border-b border-border px-4 py-3 text-sm font-semibold text-text-primary">{title}</h3>{rows.length === 0 ? <div className="p-4"><Empty t={t} /></div> : <div className="overflow-x-auto"><table className="w-full min-w-[720px] text-left text-[11px]"><thead className="text-text-tertiary"><tr><th className="px-3 py-2">{t('usage.name')}</th><th>{t('usage.tokenShare')}</th><th>{t('usage.requestShare')}</th><th>{t('usage.prompt')}</th><th>{t('usage.completion')}</th><th>{t('usage.thinking')}</th><th>{t('usage.cacheHit')}</th><th>{t('usage.avgRequest')}</th><th>{t('usage.avgTurn')}</th><th>{t('usage.success')}</th><th className="pr-3">{t('usage.cost')}</th></tr></thead><tbody>{rows.map((row) => { const cacheTotal = row.cacheReadTokens + row.cacheMissTokens; return <tr key={row.key} className="border-t border-border/60 text-text-secondary"><td className="max-w-48 truncate px-3 py-2 font-medium text-text-primary" title={row.key}>{operationLabel(row.key, t)}</td><td>{percent(row.totalTokens, totals.totalTokens)}</td><td>{percent(row.requestCount, totals.requestCount)}</td><td>{formatNumber(row.promptTokens, locale)}</td><td>{formatNumber(row.completionTokens, locale)}</td><td>{formatNumber(row.thinkingTokens, locale)}</td><td>{cacheTotal ? `${row.cacheReadTokens / cacheTotal * 100 | 0}%` : '—'}</td><td>{formatNumber(row.requestCount ? row.totalTokens / row.requestCount : 0, locale)}</td><td>{row.turnCount ? formatNumber(row.totalTokens / row.turnCount, locale) : '—'}</td><td>{row.requestCount ? `${(row.successCount / row.requestCount * 100).toFixed(1)}%` : '—'}</td><td className="pr-3">{row.estimatedCostMicros == null ? t('usage.unknown') : `$${(row.estimatedCostMicros / 1_000_000).toFixed(4)}`}</td></tr>; })}</tbody></table></div>}</section>; }

function rangeFilter(preset: Exclude<RangePreset, 'custom'> | RangePreset): api.UsageAnalyticsFilter { const now = new Date(); if (preset === 'all' || preset === 'custom') return {}; const start = new Date(now); if (preset === 'today') start.setHours(0, 0, 0, 0); else if (preset === 'year') { start.setMonth(0, 1); start.setHours(0, 0, 0, 0); } else start.setDate(start.getDate() - Number.parseInt(preset, 10)); return { startAt: start.toISOString() }; }
function contributionActivityFilter(): api.UsageAnalyticsFilter { const now = new Date(); const end = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() + 1)); const start = new Date(end.getTime() - 365 * 24 * 60 * 60 * 1000); return { startAt: start.toISOString(), endAt: end.toISOString(), timeBucket: 'day' }; }
function rangeLabel(preset: RangePreset, t: Translate): string { return t((`usage.range.${preset}`) as Parameters<Translate>[0]); }
function formatNumber(value: number, locale: string): string { return new Intl.NumberFormat(locale, { maximumFractionDigits: value < 100 ? 1 : 0, notation: value >= 1_000_000 ? 'compact' : 'standard' }).format(value); }
function percent(value: number, total: number): string { return total ? `${(value / total * 100).toFixed(1)}%` : '—'; }
function formatCost(totals: api.UsageTotals, t: Translate): string { return totals.estimatedCostMicros == null ? t('usage.unknown') : `${totals.currency ?? 'USD'} ${(totals.estimatedCostMicros / 1_000_000).toFixed(4)}`; }
function operationLabel(value: string, t: Translate): string { const key = ({ agent_main: 'mainAgent', subagent: 'subagent', summarization: 'summarization', compaction: 'compaction', conversation_title: 'titleGeneration', judge: 'judge', dreaming: 'dreaming', evolution: 'evolution', quality_evaluation: 'qualityEvaluation', workflow: 'workflow', other: 'other', legacy_unclassified: 'legacyUnclassified' } as Record<string, string>)[value]; return key ? t((`usage.operation.${key}`) as Parameters<Translate>[0]) : value; }
