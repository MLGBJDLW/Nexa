import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ArchiveRestore, Check, ChevronDown, Gauge, Loader2, RotateCcw, Sparkles, X } from 'lucide-react';
import { NexaPopover, NexaPopoverAnchor, NexaPopoverContent } from '../ui/overlay';
import { useTranslation } from '../../i18n';
import * as api from '../../lib/api';
import type { AgentConfig } from '../../types/conversation';

interface Props {
  config: AgentConfig;
  usedTokens?: number;
  isStreaming: boolean;
  isCompacting: boolean;
  onCompact?: () => void;
  onSaved: (snapshot: api.ModelContextPolicySnapshot) => void;
  children: (open: () => void) => ReactNode;
}

const numberLabel = (value: number) => value >= 1_000_000
  ? `${Number((value / 1_000_000).toFixed(2))}M`
  : value >= 1_000 ? `${Number((value / 1_000).toFixed(1))}K` : String(value);

export function ContextPolicyPopover({ config, usedTokens = 0, isStreaming, isCompacting, onCompact, onSaved, children }: Props) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [snapshot, setSnapshot] = useState<api.ModelContextPolicySnapshot | null>(null);
  const [automatic, setAutomatic] = useState(true);
  const [capacityText, setCapacityText] = useState('');
  const [compactPercent, setCompactPercent] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [saved, setSaved] = useState(false);
  const identity = `${config.id}:${config.baseUrl}:${config.model}`;
  const identityRef = useRef(identity);
  identityRef.current = identity;
  const generation = useRef(0);

  useEffect(() => { setOpen(false); setSnapshot(null); setError(''); }, [identity]);
  useEffect(() => {
    if (!open) return;
    const ticket = ++generation.current;
    setLoading(true);
    setError('');
    setSaved(false);
    void api.getModelContextPolicy(config.id, config.model).then(value => {
      if (ticket !== generation.current || identityRef.current !== identity) return;
      if (!value) throw new Error('No policy snapshot');
      setSnapshot(value);
      setAutomatic(value.policy.contextWindow == null);
      setCapacityText(String(value.policy.contextWindow ?? value.modelLimit ?? 128_000));
      setCompactPercent(value.policy.autoCompactPercent);
    }).catch(() => {
      if (ticket === generation.current) setError(t('chat.contextPolicyLoadFailed'));
    }).finally(() => {
      if (ticket === generation.current) setLoading(false);
    });
    return () => { generation.current += 1; };
  }, [open, identity, config.id, config.model, t]);

  const capacity = automatic ? snapshot?.modelLimit ?? null : Number(capacityText);
  const percent = compactPercent ?? snapshot?.defaultCompactPercent ?? 90;
  const invalid = !automatic && (!Number.isInteger(capacity) || !capacity || capacity < 8192 || capacity > (snapshot?.modelLimit ?? 16_777_216));
  const reserveTarget = snapshot?.responseReserveTarget ?? snapshot?.responseReserve ?? 16_384;
  const automaticReserve = snapshot?.responseReserveIsAutomatic ?? config.maxTokens == null;
  const responseReserve = capacity ? Math.min(reserveTarget, automaticReserve ? Math.max(1, Math.floor(capacity / 2)) : capacity) : 0;
  const safetyReserve = capacity ? capacity <= 8192 ? 256 : Math.min(8192, Math.max(1024, Math.floor(capacity / 25))) : 0;
  const promptBudget = capacity ? Math.max(0, capacity - responseReserve - safetyReserve) : 0;
  const trigger = Math.floor(promptBudget * percent / 100);
  const policy: api.ModelContextPolicy = { contextWindow: automatic ? null : Number(capacityText), autoCompactPercent: compactPercent };
  const dirty = snapshot && JSON.stringify(policy) !== JSON.stringify(snapshot.policy);
  const managed = snapshot?.managedByProvider ?? ['github_copilot', 'openai_codex'].includes(config.provider);
  const canSave = Boolean(snapshot && !managed && !invalid && !saving && !loading);
  const progress = capacity ? Math.min(100, Math.max(0, usedTokens / capacity * 100)) : 0;
  const triggerPosition = capacity ? trigger / capacity * 100 : 0;
  const reservePercent = capacity ? (responseReserve + safetyReserve) / capacity * 100 : 0;
  const touch = () => { setSaved(false); setError(''); };

  const save = async (): Promise<boolean> => {
    if (!canSave) return false;
    const owner = identity;
    setSaving(true);
    setError('');
    try {
      const next = await api.saveModelContextPolicy(config.id, config.model, policy);
      if (identityRef.current !== owner) return false;
      setSnapshot(next);
      setSaved(true);
      onSaved(next);
      return true;
    } catch {
      if (identityRef.current === owner) setError(t('chat.contextPolicySaveFailed'));
      return false;
    } finally {
      if (identityRef.current === owner) setSaving(false);
    }
  };

  const optionClass = (active: boolean) => `min-w-0 rounded-lg border px-3 py-2 text-xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 ${active ? 'border-accent/45 bg-accent/10 text-accent' : 'border-border/65 bg-surface-1 text-text-secondary hover:bg-surface-2'}`;

  return <NexaPopover open={open} onOpenChange={setOpen}>
    <NexaPopoverAnchor asChild><div className="ml-auto shrink-0">{children(() => setOpen(true))}</div></NexaPopoverAnchor>
    <NexaPopoverContent
      align="end" side="top" collisionPadding={12}
      aria-label={t('chat.contextPolicyTitle')}
      data-testid="context-policy-panel"
      className="flex w-[min(25rem,calc(100vw-1.5rem))] max-h-[min(42rem,var(--radix-popover-content-available-height))] flex-col overflow-hidden rounded-2xl border border-border/75 bg-surface-0 text-text-primary shadow-2xl"
      style={{ overflow: 'hidden' }}
    >
      <div className="flex shrink-0 items-start gap-3 border-b border-border/60 px-5 py-4">
        <span className="rounded-xl bg-accent/10 p-2 text-accent"><Gauge className="h-5 w-5" /></span>
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold">{t('chat.contextPolicyTitle')}</h2>
          <p className="mt-1 truncate text-xs text-text-tertiary" title={config.model}>{config.model}</p>
        </div>
        <button type="button" onClick={() => setOpen(false)} aria-label={t('common.close')} className="rounded-md p-1 text-text-tertiary hover:bg-surface-2"><X className="h-4 w-4" /></button>
      </div>
      {loading ? <div role="status" className="flex items-center justify-center gap-2 p-10 text-sm text-text-tertiary"><Loader2 className="h-4 w-4 animate-spin" />{t('common.loading')}</div> : snapshot ? <>
        {managed ? <div className="space-y-3 p-5"><Sparkles className="h-6 w-6 text-accent" /><h3 className="text-sm font-medium">{t('chat.contextPolicyManagedTitle')}</h3><p className="text-xs leading-6 text-text-secondary">{t('chat.contextPolicyManaged')}</p></div> : <>
          <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-5 py-4">
            <section className="rounded-xl border border-border/50 bg-surface-1/65 p-3.5" aria-label={t('chat.contextPolicyPreview')}>
              <div className="flex items-baseline justify-between gap-2">
                <span className="text-xs text-text-secondary">{t('chat.contextPolicyUsed')}</span>
                <span className="text-lg font-semibold tabular-nums">{numberLabel(usedTokens)}<span className="ml-1.5 text-xs font-normal text-text-tertiary">/ {capacity && !invalid ? numberLabel(capacity) : '—'}</span></span>
              </div>
              <div className="relative mb-2 mt-4 h-3 overflow-visible rounded-full bg-surface-3" data-testid="context-policy-budget-bar">
                <span className="absolute inset-y-0 left-0 rounded-full bg-accent transition-[width] motion-reduce:transition-none" style={{ width: `${progress}%` }} />
                <span className="absolute inset-y-0 right-0 rounded-r-full bg-text-tertiary/20" style={{ width: `${reservePercent}%` }} />
                {capacity && !invalid ? <span className="absolute -inset-y-1 border-l-2 border-dashed border-warning" style={{ left: `${triggerPosition}%` }} title={t('chat.contextPolicyTriggerAt', { tokens: numberLabel(trigger) })} /> : null}
              </div>
              <div className="flex flex-wrap justify-between gap-x-3 gap-y-1 text-[10px] text-text-tertiary">
                <span className="flex items-center gap-1.5"><span className="h-1.5 w-1.5 rounded-full bg-accent" />{t('chat.contextPolicyUsed')}</span>
                <span className="flex items-center gap-1.5"><span className="h-2 border-l-2 border-dashed border-warning" />{t('chat.contextPolicyCompact')}</span>
                <span>{t('chat.contextPolicyReserved', { tokens: numberLabel(responseReserve + safetyReserve) })}</span>
              </div>
            </section>

            <section className="space-y-3">
              <div className="flex items-center justify-between gap-2"><h3 className="text-xs font-semibold">{t('chat.contextPolicyCapacity')}</h3><span className="text-[11px] text-text-tertiary">{snapshot.modelLimit ? t('chat.contextPolicyModelLimit', { tokens: numberLabel(snapshot.modelLimit) }) : t('chat.contextPolicyUnknownLimit')}</span></div>
              <div className="grid grid-cols-2 gap-2">
                <button type="button" aria-pressed={automatic} className={optionClass(automatic)} onClick={() => { setAutomatic(true); touch(); }}><span className="flex items-center justify-center gap-1.5"><Sparkles className="h-3.5 w-3.5" />{t('chat.contextPolicyAutomatic')}</span></button>
                <button type="button" aria-pressed={!automatic} className={optionClass(!automatic)} onClick={() => { setAutomatic(false); touch(); }}>{t('chat.contextPolicyCustom')}</button>
              </div>
              {!automatic && <div className="space-y-3" data-testid="context-policy-custom">
                <div className="flex flex-wrap gap-1.5">{[64_000, 128_000, 256_000, 512_000].filter(value => !snapshot.modelLimit || value <= snapshot.modelLimit).map(value => <button key={value} type="button" className={`rounded-md px-2 py-1 text-[11px] ${Number(capacityText) === value ? 'bg-accent/12 text-accent' : 'bg-surface-2 text-text-secondary hover:bg-surface-3'}`} onClick={() => { setCapacityText(String(value)); touch(); }}>{numberLabel(value)}</button>)}{snapshot.modelLimit && <button type="button" className="rounded-md bg-surface-2 px-2 py-1 text-[11px] text-text-secondary" onClick={() => { setCapacityText(String(snapshot.modelLimit)); touch(); }}>{t('chat.contextPolicyMaximum')}</button>}</div>
                <label className="flex items-center gap-2 rounded-lg border border-border bg-surface-1 px-3 py-2 focus-within:border-accent/60"><input data-testid="context-policy-capacity" aria-label={t('chat.contextPolicyCapacity')} type="number" min={8192} max={snapshot.modelLimit ?? 16_777_216} step={1} value={capacityText} onChange={event => { setCapacityText(event.target.value); touch(); }} className="min-w-0 flex-1 bg-transparent text-sm tabular-nums outline-none" /><span className="text-xs text-text-tertiary">tokens</span></label>
                {invalid && <p role="alert" className="text-xs text-danger">{t('chat.contextPolicyInvalidCapacity', { limit: snapshot.modelLimit ? numberLabel(snapshot.modelLimit) : '16M' })}</p>}
              </div>}
            </section>

            <section className="space-y-3 border-t border-border/60 pt-4">
              <div className="flex items-center justify-between gap-2"><h3 className="text-xs font-semibold">{t('chat.contextPolicyTiming')}</h3><span className="text-base font-semibold tabular-nums text-accent">{percent}<span className="ml-0.5 text-xs">%</span></span></div>
              <div className="grid grid-cols-3 gap-2">{([{ value: 65, label: 'chat.contextPolicyEarlier' }, { value: null, label: 'chat.contextPolicyBalanced' }, { value: 95, label: 'chat.contextPolicyLater' }] as const).map(item => <button type="button" key={item.label} aria-pressed={compactPercent === item.value} className={optionClass(compactPercent === item.value)} onClick={() => { setCompactPercent(item.value); touch(); }}>{t(item.label)}</button>)}</div>
              <input data-testid="context-policy-threshold" aria-label={t('chat.contextPolicyTiming')} type="range" min={60} max={95} step={1} value={percent} onChange={event => { setCompactPercent(Number(event.target.value)); touch(); }} className="block h-5 w-full cursor-pointer accent-accent" />
              <div className="flex justify-between text-[10px] text-text-tertiary"><span>{t('chat.contextPolicyMoreRoom')}</span><span>{t('chat.contextPolicyMoreHistory')}</span></div>
              <p className="text-xs leading-5 text-text-secondary">{capacity && !invalid ? t('chat.contextPolicyTriggerAt', { tokens: numberLabel(trigger) }) : t('chat.contextPolicyUnknownTrigger')}</p>
              <details className="group text-[11px] text-text-tertiary"><summary className="flex cursor-pointer list-none items-center gap-1"><ChevronDown className="h-3 w-3 transition-transform group-open:rotate-180" />{t('chat.contextPolicyHowCalculated')}</summary><p className="mt-2 leading-5">{t('chat.contextPolicyCalculation', { input: numberLabel(promptBudget), reply: numberLabel(responseReserve), safety: numberLabel(safetyReserve) })}</p></details>
            </section>
          </div>
          <div className="shrink-0 space-y-3 border-t border-border/60 bg-surface-1 px-5 py-4">
            <p className="text-[11px] leading-5 text-text-tertiary">{t('chat.contextPolicyScope')}</p>
            {error && <p role="alert" className="text-xs text-danger">{error}</p>}
            {saved && <p role="status" className="flex items-center gap-1.5 text-xs text-accent"><Check className="h-3.5 w-3.5" />{t('chat.contextPolicySaved')}</p>}
            <div className="flex items-center justify-between gap-2">
              <button type="button" aria-label={t('chat.contextPolicyReset')} title={t('chat.contextPolicyReset')} onClick={() => { setAutomatic(true); setCompactPercent(null); touch(); }} className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-lg text-text-tertiary hover:bg-surface-2 hover:text-text-primary"><RotateCcw className="h-4 w-4" /></button>
              {onCompact && <button type="button" data-testid="context-policy-compact-now" disabled={!canSave || isStreaming || isCompacting} onClick={async () => { if (dirty && !await save()) return; onCompact(); }} className="mr-auto inline-flex min-h-9 min-w-0 items-center justify-center gap-1.5 rounded-lg border border-border/70 px-2 py-1 text-xs text-text-secondary hover:bg-surface-2 disabled:opacity-40"><ArchiveRestore className="h-3.5 w-3.5 shrink-0" /><span>{isCompacting ? t('chat.compacting') : dirty ? t('chat.contextPolicySaveCompact') : t('chat.compactNow')}</span></button>}
              <button type="button" data-testid="context-policy-save" disabled={!canSave || !dirty} onClick={() => void save()} className="inline-flex min-h-9 items-center justify-center gap-1.5 rounded-lg bg-accent px-4 text-xs font-semibold text-white transition-opacity hover:opacity-90 disabled:opacity-40">{saving && <Loader2 className="h-3.5 w-3.5 animate-spin" />}{t('chat.contextPolicyApply')}</button>
            </div>
          </div>
        </>}
      </> : <div role="alert" className="p-5 text-xs text-danger">{error}</div>}
    </NexaPopoverContent>
  </NexaPopover>;
}
