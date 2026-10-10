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
      <div className="nexa-intensity-meter" aria-hidden="true">
        {Array.from({ length: 32 }, (_, index) => <span key={index} data-active={index / 31 <= ratio} style={{ '--bar-height': `${20 + index * 1.3}px` } as CSSProperties} />)}
      </div>
      <input type="range" min={0} max={options.length - 1} step={1} value={current} disabled={options.length < 2}
        aria-label={t('settings.reasoningEffort')} aria-valuetext={options[current].label}
        onChange={event => setDraft(Number(event.target.value))}
        onPointerUp={event => commit(Number(event.currentTarget.value))}
        onPointerCancel={() => setDraft(selected)}
        onKeyUp={event => { if (['ArrowLeft','ArrowRight','ArrowUp','ArrowDown','Home','End','PageUp','PageDown'].includes(event.key)) commit(Number(event.currentTarget.value)); }}
        className="nexa-intensity-range" />
      <div className="mt-2 flex justify-between gap-2 text-[10px] text-text-tertiary">
        <span>{options[0].label}</span><span>{options[options.length - 1].label}</span>
      </div>
      <div className="mt-3 flex flex-wrap gap-1">
        {options.map((option, index) => <button type="button" key={option.key} onClick={() => commit(index)}
          aria-pressed={current === index} className={`rounded-md px-2 py-1 text-[10px] transition-colors focus-visible:outline-2 focus-visible:outline-accent ${current === index ? 'bg-accent-subtle text-accent' : 'text-text-secondary hover:bg-surface-2'}`}>
          {option.label}
        </button>)}
      </div>
    </div>}
  </div>;
}
