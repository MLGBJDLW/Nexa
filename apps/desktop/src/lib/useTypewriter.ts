import { useRef, useEffect, useState } from 'react';

/**
 * Gradually reveals `sourceText` with a typewriter effect, advancing by
 * word-boundaries so markdown tokens are never split mid-tag.
 *
 * While `isActive` is true the hook ticks forward; once streaming ends
 * (`isActive` → false) the full text is revealed instantly.
 */
export function useTypewriter(
  sourceText: string,
  isActive: boolean,
  options?: { charsPerTick?: number; intervalMs?: number },
): string {
  const charsPerTick = options?.charsPerTick ?? 5;
  const intervalMs = options?.intervalMs ?? 30;

  const [displayed, setDisplayed] = useState('');
  const revealIdx = useRef(0);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // When streaming finishes, flush everything immediately.
  useEffect(() => {
    if (!isActive) {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
      revealIdx.current = sourceText.length;
      setDisplayed(sourceText);
    }
  }, [isActive, sourceText]);

  // Tick-based reveal while active.
  useEffect(() => {
    if (!isActive) return;

    // If there's nothing to show yet, reset.
    if (!sourceText) {
      revealIdx.current = 0;
      setDisplayed('');
      return;
    }

    // Start interval if not already running.
    if (timerRef.current) return;

    timerRef.current = setInterval(() => {
      const src = sourceText; // closed-over in state setter below anyway
      setDisplayed((prev) => {
        let idx = revealIdx.current;
        if (idx >= src.length) return prev;

        const gap = src.length - idx;
        // Dynamic acceleration when gap is large.
        const step = gap > 100 ? Math.max(charsPerTick, Math.floor(gap / 8)) : charsPerTick;
        let target = Math.min(idx + step, src.length);

        // Snap forward to the next word boundary to avoid splitting markdown.
        while (target < src.length && !/\s/.test(src[target])) {
          target++;
        }

        revealIdx.current = target;
        return src.slice(0, target);
      });
    }, intervalMs);

    return () => {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
    };
    // Re-run when sourceText grows so the interval captures the new length.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isActive, sourceText, charsPerTick, intervalMs]);

  // Cleanup on unmount.
  useEffect(() => {
    return () => {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
    };
  }, []);

  return displayed;
}
