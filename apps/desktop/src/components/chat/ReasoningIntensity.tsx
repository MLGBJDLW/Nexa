import { useEffect, useId, useRef, useState, type ReactNode, type CSSProperties, type PointerEvent } from 'react';
import { AnimatePresence, motion, useReducedMotion, useSpring, useTransform } from 'framer-motion';
import { List, SlidersHorizontal } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { updateDisplayPreferences, useDisplayPreferences } from '../../lib/displayPreferences';
import './reasoningIntensity.css';

export interface IntensityOption { key: string; label: string }

// Keep effort colors stable when a provider offers only a subset of levels.
const EFFORT_COLORS: Record<string, string> = {
  'native-default': 'var(--color-text-tertiary)',
  off: 'var(--color-text-tertiary)',
  'effort:none': 'var(--color-text-tertiary)',
  'effort:minimal': '#7199a6',
  'effort:low': '#68a99a',
  'effort:medium': '#c2a167',
  'effort:high': '#c9827e',
  'effort:xhigh': '#a78ac4',
  'effort:max': '#b776aa',
  'effort:ultra': '#cc7fa2',
  on: 'var(--color-accent)',
};
const SPRING = { type: 'spring' as const, stiffness: 380, damping: 32, mass: 0.6 };
const RANGE_KEYS = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'PageUp', 'PageDown'];
const HELIX_PERIOD = 140;
const helixY = (x: number, phase = 0) => 16 + Math.sin(x / HELIX_PERIOD * Math.PI * 2 + phase) * 9;
// One extra period lets the browser translate a fixed drawing seamlessly. The
// previous SMIL paths and endpoints rebuilt SVG geometry on every frame.
const helixPath = (phase: number) => Array.from({ length: 73 }, (_, index) => {
  const x = index / 48 * 280;
  return `${index ? 'L' : 'M'}${x.toFixed(2)},${helixY(x, phase).toFixed(2)}`;
}).join(' ');
const STRANDS = [0, Math.PI].map(helixPath);
const RUNGS = Array.from({ length: 15 }, (_, index) => {
  const x = 10 + index * 260 / 14;
  return { left: `${x / 280 * 100}%`, delay: `${-x / HELIX_PERIOD * 8}s`, scale: Math.sin(x / HELIX_PERIOD * Math.PI * 2) };
});

function ReasoningBond({ reducedMotion }: { reducedMotion: boolean }) {
  return <span className="nexa-intensity-bond" data-reduced-motion={reducedMotion} aria-hidden="true">
    {RUNGS.map(({ left, delay, scale }, index) => <span key={index} className="nexa-intensity-rung"
      style={{ left, animationDelay: delay, transform: `scaleY(${scale})` }} />)}
    <svg className="nexa-intensity-strands" viewBox="0 0 420 32" preserveAspectRatio="none">
      {STRANDS.map((path, index) => <path key={index} d={path} />)}
    </svg>
    {RUNGS.filter((_, index) => index % 4 === 1).map(({ left, delay, scale }, index) => <span key={index}
      className="nexa-intensity-spark" style={{ left, animationDelay: delay, transform: `translateY(${scale * 5.0625}px)` }} />)}
  </span>;
}

export function ReasoningIntensity({ options, selectedKey, onSelect, children }: {
  options: IntensityOption[];
  selectedKey: string;
  onSelect: (key: string) => void | boolean | Promise<void | boolean>;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const { reasoningControl } = useDisplayPreferences();
  const reducedMotion = !!useReducedMotion();
  const animationId = useId();
  const selected = Math.max(0, options.findIndex(option => option.key === selectedKey));
  const lastIndex = Math.max(1, options.length - 1);
  const optionsKey = options.map(option => option.key).join('\0');
  const [draft, setDraft] = useState(selected);
  const [saving, setSaving] = useState(false);
  const pendingSaves = useRef(0);
  const latestRequest = useRef(0);
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const drag = useRef<{ pointerId: number; startX: number; moved: boolean } | null>(null);
  const keyboard = useRef(false);
  const progress = useSpring(selected / lastIndex, SPRING);
  const thumbPosition = useTransform(progress, value => `calc(13px + (100% - 26px) * ${value})`);
  useEffect(() => {
    if (pendingSaves.current > 0) return;
    drag.current = null;
    setDraft(selected);
    if (reducedMotion) progress.jump(selected / lastIndex);
    else progress.set(selected / lastIndex);
  }, [selected, selectedKey, optionsKey, lastIndex, progress, reducedMotion]);
  if (!options.length) return <>{children}</>;
  const current = Math.min(draft, options.length - 1);
  const ratio = current / lastIndex;
  const color = EFFORT_COLORS[options[current].key]
    ?? `color-mix(in srgb, #68a99a ${Math.round((1 - ratio) * 100)}%, #b776aa)`;
  const moveTo = (value: number) => {
    setDraft(value);
    if (reducedMotion) progress.jump(value / lastIndex);
    else progress.set(value / lastIndex);
  };
  const commit = async (value: number) => {
    const request = ++latestRequest.current;
    const index = Math.min(Math.max(0, Math.round(value)), options.length - 1);
    moveTo(index);
    pendingSaves.current++;
    setSaving(true);
    try {
      const persisted = await onSelect(options[index].key);
      if (request === latestRequest.current && persisted === false) moveTo(selectedRef.current);
    } catch {
      if (request === latestRequest.current) moveTo(selectedRef.current);
    } finally {
      pendingSaves.current--;
      if (request === latestRequest.current) setSaving(false);
    }
  };
  const pointerRatio = (event: PointerEvent<HTMLInputElement>) => {
    const box = event.currentTarget.getBoundingClientRect();
    return Math.min(1, Math.max(0, (event.clientX - box.left - 13) / Math.max(1, box.width - 26)));
  };
  const transition = reducedMotion ? { duration: 0 } : { duration: 0.18 };
  return <div>
    <div role="group" aria-label={t('chat.reasoningControlStyle')} className="nexa-intensity-switch mb-2 flex rounded-lg border border-border/60 bg-surface-1 p-0.5">
      {(['list', 'slider'] as const).map(style => {
        const Icon = style === 'list' ? List : SlidersHorizontal;
        return <button key={style} type="button" aria-pressed={reasoningControl === style}
          onClick={() => updateDisplayPreferences({ reasoningControl: style })}
          className="relative isolate flex h-7 flex-1 items-center justify-center gap-1.5 rounded-md text-[11px] text-text-secondary transition-colors focus-visible:outline-2 focus-visible:outline-accent">
          {reasoningControl === style && <motion.span className="absolute inset-0 -z-10 rounded-md bg-surface-3 shadow-sm" layoutId={reducedMotion ? undefined : `${animationId}-style`} transition={reducedMotion ? transition : SPRING} />}
          <Icon size={12} aria-hidden="true" />{t(style === 'list' ? 'chat.reasoningList' : 'chat.reasoningSlider')}
        </button>;
      })}
    </div>
    <AnimatePresence initial={false} mode="wait">
      <motion.div key={reasoningControl} initial={reducedMotion ? false : { opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: reducedMotion ? 0 : -3 }} transition={transition}>
        {reasoningControl === 'list' ? children : <div className="nexa-intensity" aria-busy={saving} style={{ '--intensity-color': color } as CSSProperties}>
          <div className="mb-1.5 flex items-baseline justify-between gap-2">
            <span className="text-[11px] text-text-tertiary">{t('settings.reasoningEffort')}</span>
            <output className="nexa-intensity-label text-sm font-semibold text-text-primary" data-testid="reasoning-slider-label">
              <AnimatePresence initial={false} mode="wait">
                <motion.span key={options[current].key} initial={reducedMotion ? false : { opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: reducedMotion ? 0 : -4 }} transition={{ duration: reducedMotion ? 0 : 0.1 }}>{options[current].label}</motion.span>
              </AnimatePresence>
            </output>
          </div>
          <div className="nexa-intensity-rail">
            <div className="nexa-intensity-track" aria-hidden="true">
              <motion.div className="nexa-intensity-fill" style={{ width: thumbPosition }} />
              <ReasoningBond reducedMotion={reducedMotion} />
              <div className="nexa-intensity-sheen" />
            </div>
            <div className="nexa-intensity-stops" aria-hidden="true">
              {options.map((option, index) => <span key={option.key} data-active={index < current} data-selected={index === current} />)}
            </div>
            <motion.div className="nexa-intensity-thumb" style={{ left: thumbPosition, x: '-50%' }} aria-hidden="true"><span /></motion.div>
            <input type="range" min={0} max={options.length - 1} step={1} value={current} disabled={options.length < 2}
              aria-label={t('settings.reasoningEffort')} aria-valuetext={options[current].label}
              onChange={event => {
                const value = Number(event.target.value);
                if (drag.current) setDraft(value);
                else if (keyboard.current) moveTo(value);
                else commit(value);
              }}
              onPointerDown={event => {
                if (event.button !== 0) return;
                event.currentTarget.setPointerCapture(event.pointerId);
                drag.current = { pointerId: event.pointerId, startX: event.clientX, moved: false };
                moveTo(Math.round(pointerRatio(event) * lastIndex));
              }}
              onPointerMove={event => {
                if (drag.current?.pointerId !== event.pointerId) return;
                if (Math.abs(event.clientX - drag.current.startX) > 2) drag.current.moved = true;
                if (!drag.current.moved) return;
                const value = pointerRatio(event);
                progress.jump(value);
                setDraft(Math.round(value * lastIndex));
              }}
              onPointerUp={event => {
                if (drag.current?.pointerId !== event.pointerId) return;
                drag.current = null;
                commit(pointerRatio(event) * lastIndex);
              }}
              onPointerCancel={() => { drag.current = null; moveTo(selected); }}
              onKeyDown={event => { if (RANGE_KEYS.includes(event.key)) keyboard.current = true; }}
              onKeyUp={event => { if (RANGE_KEYS.includes(event.key)) { keyboard.current = false; commit(Number(event.currentTarget.value)); } }}
              onBlur={() => { keyboard.current = false; }}
              className="nexa-intensity-range" />
          </div>
          <div className="nexa-intensity-options mt-1.5 flex flex-wrap gap-1">
            {options.map((option, index) => <motion.button type="button" key={option.key} onClick={() => commit(index)}
              whileHover={reducedMotion ? undefined : { y: -1 }} whileTap={reducedMotion ? undefined : { scale: 0.97 }}
              aria-pressed={current === index} className="relative isolate rounded-full px-2 py-1 text-[10px] text-text-secondary transition-colors focus-visible:outline-2 focus-visible:outline-accent">
              {current === index && <motion.span className="nexa-intensity-option-highlight" layoutId={reducedMotion ? undefined : `${animationId}-option`} transition={reducedMotion ? transition : SPRING} />}
              {option.label}
            </motion.button>)}
          </div>
        </div>}
      </motion.div>
    </AnimatePresence>
  </div>;
}
