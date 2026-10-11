import { useEffect, useState, type ReactNode, type CSSProperties } from 'react';
import { List, SlidersHorizontal } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { updateDisplayPreferences, useDisplayPreferences } from '../../lib/displayPreferences';
import './reasoningIntensity.css';

export interface IntensityOption { key: string; label: string }

export function ReasoningIntensity({ options, selectedKey, onSelect, children }: {
  options: IntensityOption[];
  selectedKey: string;
  onSelect: (key: string) => void;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const { reasoningControl } = useDisplayPreferences();
  const selected = Math.max(0, options.findIndex(option => option.key === selectedKey));
  const [draft, setDraft] = useState(selected);
  useEffect(() => setDraft(selected), [selected, selectedKey, options.length]);
  if (!options.length) return <>{children}</>;
  const current = Math.min(draft, options.length - 1);
  const ratio = options.length > 1 ? current / (options.length - 1) : 1;
  const commit = (value: number) => onSelect(options[Math.min(Math.max(0, value), options.length - 1)].key);
  // Continuous strands echo Nexa's bond motif; effort adds tension, not a token scale.
  const strand = (phase: number) => Array.from({ length: 81 }, (_, index) => {
    const x = index / 80;
    const y = 24 + Math.sin(x * Math.PI * (2 + ratio * 4) + phase) * Math.sin(x * Math.PI) * (3 + ratio * 11);
    return `${index ? 'L' : 'M'}${(x * 280).toFixed(2)},${y.toFixed(2)}`;
  }).join(' ');
  return <div>
    <div role="group" aria-label={t('chat.reasoningControlStyle')} className="mb-3 flex rounded-lg border border-border/60 bg-surface-1 p-0.5">
      {(['list', 'slider'] as const).map(style => {
        const Icon = style === 'list' ? List : SlidersHorizontal;
        return <button key={style} type="button" aria-pressed={reasoningControl === style}
          onClick={() => updateDisplayPreferences({ reasoningControl: style })}
          className={`flex h-7 flex-1 items-center justify-center gap-1.5 rounded-md text-[11px] transition-colors focus-visible:outline-2 focus-visible:outline-accent ${reasoningControl === style ? 'bg-surface-3 text-text-primary shadow-sm' : 'text-text-tertiary hover:text-text-primary'}`}>
          <Icon size={12} aria-hidden="true" />{t(style === 'list' ? 'chat.reasoningList' : 'chat.reasoningSlider')}
        </button>;
      })}
    </div>
    {reasoningControl === 'list' ? children : <div className="nexa-intensity" style={{ '--intensity': ratio } as CSSProperties}>
      <div className="mb-3 flex items-baseline justify-between gap-2">
        <span className="text-[11px] text-text-tertiary">{t('settings.reasoningEffort')}</span>
        <output className="text-sm font-semibold text-accent" data-testid="reasoning-slider-label">{options[current].label}</output>
      </div>
      <div className="nexa-intensity-rail">
        <svg className="nexa-intensity-bond" viewBox="0 0 280 48" preserveAspectRatio="none" aria-hidden="true">
          <path d={strand(0)} /><path d={strand(Math.PI)} />
        </svg>
        <input type="range" min={0} max={options.length - 1} step={1} value={current} disabled={options.length < 2}
          aria-label={t('settings.reasoningEffort')} aria-valuetext={options[current].label}
          onChange={event => setDraft(Number(event.target.value))}
          onPointerDown={event => event.currentTarget.setPointerCapture(event.pointerId)}
          onPointerUp={event => commit(Number(event.currentTarget.value))}
          onPointerCancel={() => setDraft(selected)}
          onKeyUp={event => { if (['ArrowLeft','ArrowRight','ArrowUp','ArrowDown','Home','End','PageUp','PageDown'].includes(event.key)) commit(Number(event.currentTarget.value)); }}
          className="nexa-intensity-range" />
        <div className="nexa-intensity-stops" aria-hidden="true">
          {options.map((option, index) => <span key={option.key} data-active={index <= current} />)}
        </div>
      </div>
      <div className="mt-2 flex justify-between gap-2 text-[10px] text-text-tertiary">
        <span>{options[0].label}</span><span>{options[options.length - 1].label}</span>
      </div>
      <div className="nexa-intensity-options mt-3 flex flex-wrap gap-1">
        {options.map((option, index) => <button type="button" key={option.key} onClick={() => commit(index)}
          aria-pressed={current === index} className={`rounded-md px-2 py-1 text-[10px] transition-colors focus-visible:outline-2 focus-visible:outline-accent ${current === index ? 'bg-accent-subtle text-accent' : 'text-text-secondary hover:bg-surface-2'}`}>
          {option.label}
        </button>)}
      </div>
    </div>}
  </div>;
}
