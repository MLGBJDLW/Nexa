import { ChevronDown, Search } from 'lucide-react';
import { useMemo, useState } from 'react';

import { useTranslation } from '../../i18n';
import { usageModelIdentity } from '../../features/usage/modelIdentity';
import type { UsageBreakdownRow } from '../../lib/api';
import { ProviderIcon, resolveProviderIconMeta } from '../../lib/providerIcons';

export function usageProviderLabel(provider: string): string {
  const meta = resolveProviderIconMeta({ provider, providerId: provider, label: provider });
  return meta.label === 'Custom provider' ? provider : meta.label;
}

export function UsageModelBreakdown({ rows }: { rows: UsageBreakdownRow[] }) {
  const { t, locale } = useTranslation();
  const [groupBy, setGroupBy] = useState<'provider' | 'model'>('provider');
  const [search, setSearch] = useState('');
  const number = useMemo(() => new Intl.NumberFormat(locale, { maximumFractionDigits: 0 }), [locale]);
  const groups = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    const grouped = new Map<string, UsageBreakdownRow[]>();
    for (const row of rows) {
      const provider = row.providerId || t('usage.unknown');
      const model = row.modelId || t('usage.unknown');
      if (query && !`${provider} ${usageProviderLabel(provider)} ${model}`.toLocaleLowerCase().includes(query)) continue;
      const key = groupBy === 'provider' ? provider : usageModelIdentity(model);
      const group = grouped.get(key) ?? [];
      group.push(row);
      grouped.set(key, group);
    }
    return [...grouped].map(([key, children]) => ({
      key,
      rows: children.sort((a, b) => b.totalTokens - a.totalTokens || a.key.localeCompare(b.key)),
      tokens: children.reduce((sum, row) => sum + row.totalTokens, 0),
      requests: children.reduce((sum, row) => sum + row.requestCount, 0),
    })).sort((a, b) => b.tokens - a.tokens || a.key.localeCompare(b.key));
  }, [rows, groupBy, search, t]);
  const totalTokens = rows.reduce((sum, row) => sum + row.totalTokens, 0);
  const detail = (label: string, value: string) => <div key={label}><dt className="text-text-tertiary">{label}</dt><dd className="mt-0.5 tabular-nums text-text-primary">{value}</dd></div>;

  return (
    <section className="min-w-0 overflow-hidden rounded-xl border border-border bg-surface-1" data-testid="usage-model-breakdown">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border p-4">
        <div>
          <h3 className="text-sm font-semibold text-text-primary">{t('usage.providerModel')}</h3>
          <p className="mt-1 text-xs text-text-tertiary">{t('usage.groupHint')}</p>
        </div>
        <div className="flex rounded-lg bg-surface-2 p-1" aria-label={t('usage.groupBy')} role="group">
          {(['provider', 'model'] as const).map((mode) => <button key={mode} type="button" aria-pressed={groupBy === mode} onClick={() => setGroupBy(mode)} className={`rounded-md px-3 py-1.5 text-xs ${groupBy === mode ? 'bg-surface-0 font-medium text-text-primary shadow-sm' : 'text-text-tertiary hover:text-text-primary'}`}>{t(mode === 'provider' ? 'usage.byProvider' : 'usage.byModel')}</button>)}
        </div>
        <label className="flex w-full items-center gap-2 rounded-lg border border-border bg-surface-0 px-3 py-2 text-text-tertiary">
          <Search size={14} aria-hidden="true" />
          <input type="search" aria-label={t('usage.searchModels')} placeholder={t('usage.searchModels')} value={search} onChange={(event) => setSearch(event.target.value)} className="min-w-0 flex-1 bg-transparent text-xs text-text-primary outline-none" />
        </label>
      </div>
      {groups.length === 0 ? <p className="p-6 text-center text-xs text-text-tertiary">{t('usage.empty')}</p> : (
        <div className="divide-y divide-border">
          {groups.map((group) => <details key={`${groupBy}:${group.key}`} open className="group/usage" data-testid="usage-model-group">
            <summary className="flex cursor-pointer list-none flex-wrap items-center gap-2 px-4 py-3 hover:bg-surface-2">
              <ChevronDown size={14} className="shrink-0 text-text-tertiary transition-transform group-not-open/usage:-rotate-90" />
              {groupBy === 'provider' && <ProviderIcon provider={group.key} providerId={group.key} label={group.key} size="sm" />}
              <span className="min-w-0 flex-1 break-all text-sm font-medium text-text-primary">{groupBy === 'provider' ? usageProviderLabel(group.key) : group.key}</span>
              <span className="rounded-md bg-surface-2 px-1.5 py-0.5 text-[10px] text-text-tertiary">{group.rows.length}</span>
              <span className="ml-auto text-right text-xs tabular-nums text-text-secondary" data-testid="usage-group-total">{number.format(group.tokens)} <span className="text-text-tertiary">{t('usage.tokens')}</span><span className="ml-3">{number.format(group.requests)} <span className="text-text-tertiary">{t('usage.requests')}</span></span></span>
            </summary>
            <div className="mx-4 mb-3 rounded-lg border border-border/70">
              <div className="grid grid-cols-[minmax(0,1fr)_5rem_4rem] gap-2 rounded-t-lg bg-surface-2/60 px-3 py-2 text-[10px] text-text-tertiary sm:grid-cols-[minmax(0,1fr)_7rem_5rem_4rem]" aria-hidden="true"><span>{t(groupBy === 'provider' ? 'usage.byModel' : 'usage.byProvider')}</span><span className="text-right">{t('usage.tokens')}</span><span className="text-right">{t('usage.requests')}</span><span className="hidden text-right sm:block">{t('usage.tokenShare')}</span></div>
              {group.rows.map((row) => {
                const cacheTotal = row.cacheReadTokens + row.cacheMissTokens;
                return <details key={row.key} className="border-t border-border/60" data-testid="usage-model-row">
                  <summary className="grid cursor-pointer list-none grid-cols-[minmax(0,1fr)_5rem_4rem] items-center gap-2 px-3 py-2.5 text-xs hover:bg-surface-2/60 sm:grid-cols-[minmax(0,1fr)_7rem_5rem_4rem]">
                    <span className="flex min-w-0 items-center gap-2"><ChevronDown size={12} className="shrink-0 text-text-tertiary" /><span className="break-all text-text-primary">{groupBy === 'provider' ? row.modelId || t('usage.unknown') : usageProviderLabel(row.providerId || t('usage.unknown'))}</span></span>
                    <span className="text-right tabular-nums text-text-secondary">{number.format(row.totalTokens)}</span>
                    <span className="text-right tabular-nums text-text-secondary">{number.format(row.requestCount)}</span>
                    <span className="hidden text-right tabular-nums text-text-tertiary sm:block">{totalTokens ? `${(row.totalTokens / totalTokens * 100).toFixed(1)}%` : '—'}</span>
                  </summary>
                  <dl className="grid grid-cols-2 gap-x-4 gap-y-3 border-t border-border/50 bg-surface-0/40 px-5 py-3 text-[11px] sm:grid-cols-4">
                    {detail(t('usage.inputTokens'), number.format(row.promptTokens))}
                    {detail(t('usage.outputTokens'), number.format(row.completionTokens))}
                    {detail(t('usage.thinkingTokens'), number.format(row.thinkingTokens))}
                    {detail(t('usage.cacheHitRate'), cacheTotal ? `${(row.cacheReadTokens / cacheTotal * 100).toFixed(1)}%` : '—')}
                    {detail(t('usage.avgRequest'), row.requestCount ? number.format(row.totalTokens / row.requestCount) : '—')}
                    {detail(t('usage.avgTurn'), row.turnCount ? number.format(row.totalTokens / row.turnCount) : '—')}
                    {detail(t('usage.success'), row.requestCount ? `${(row.successCount / row.requestCount * 100).toFixed(1)}%` : '—')}
                    {detail(t('usage.cost'), row.estimatedCostMicros == null ? t('usage.unknown') : `$${(row.estimatedCostMicros / 1_000_000).toFixed(4)}`)}
                  </dl>
                </details>;
              })}
            </div>
          </details>)}
        </div>
      )}
    </section>
  );
}
