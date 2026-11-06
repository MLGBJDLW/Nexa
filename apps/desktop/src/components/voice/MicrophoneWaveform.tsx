import { useEffect, useRef } from 'react';
import { useReducedMotion } from 'framer-motion';

import { computeWaveformBars, smoothWaveformBars, toBarHeights } from '../../features/voice/waveform';

interface MicrophoneWaveformProps {
  /** Live analyser to read from; when null the bars rest at their floor. */
  analyser: AnalyserNode | null;
  /** Native capture supplies acoustic levels without opening a WebView microphone. */
  subscribeLevel?: (update: (level: number) => void) => () => void;
  barCount?: number;
  className?: string;
  label?: string;
}

/**
 * Live microphone waveform.
 *
 * Heights are written straight to the DOM inside the animation frame so a
 * 60 fps meter never re-renders the surrounding chat input or settings form.
 */
export function MicrophoneWaveform({
  analyser,
  subscribeLevel,
  barCount = 24,
  className = '',
  label,
}: MicrophoneWaveformProps) {
  const barsRef = useRef<Array<HTMLSpanElement | null>>([]);
  const shouldReduceMotion = useReducedMotion();

  useEffect(() => {
    const bars = barsRef.current;
    if ((!analyser && !subscribeLevel) || shouldReduceMotion) {
      bars.forEach((bar) => {
        if (bar) bar.style.transform = `scaleY(${analyser ? '0.18' : '0.06'})`;
      });
      return;
    }

    if (subscribeLevel) {
      const levels = new Array<number>(barCount).fill(0);
      return subscribeLevel(level => {
        levels.shift();
        levels.push(level);
        toBarHeights(levels).forEach((height, i) => { if (bars[i]) bars[i].style.transform = `scaleY(${height.toFixed(3)})`; });
      });
    }
    if (!analyser) return;
    const samples = new Float32Array(analyser.fftSize);
    let smoothed = new Array<number>(barCount).fill(0);
    let frame = 0;

    const draw = () => {
      analyser.getFloatTimeDomainData(samples);
      smoothed = smoothWaveformBars(smoothed, computeWaveformBars(samples, barCount));
      const heights = toBarHeights(smoothed);
      for (let index = 0; index < bars.length; index += 1) {
        const bar = bars[index];
        if (bar) bar.style.transform = `scaleY(${heights[index].toFixed(3)})`;
      }
      frame = requestAnimationFrame(draw);
    };

    frame = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(frame);
  }, [analyser, subscribeLevel, barCount, shouldReduceMotion]);

  return (
    <div
      data-testid="microphone-waveform"
      data-active={analyser || subscribeLevel ? 'true' : 'false'}
      data-animated={(analyser || subscribeLevel) && !shouldReduceMotion ? 'true' : 'false'}
      role="img"
      aria-label={label}
      className={`flex h-6 items-center gap-[2px] ${className}`}
    >
      {Array.from({ length: barCount }, (_, index) => (
        <span
          key={index}
          ref={(node) => {
            barsRef.current[index] = node;
          }}
          className="h-full w-[2px] origin-center rounded-full bg-current transition-transform duration-75 ease-out"
          style={{ transform: 'scaleY(0.06)' }}
        />
      ))}
    </div>
  );
}
