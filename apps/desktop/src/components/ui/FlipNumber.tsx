import { useReducedMotionPreference as useReducedMotion } from '../../lib/uiMotion';
import { AnimatePresence, motion } from 'framer-motion';

/** Stable digit columns keep changing counters readable, with one accessible value. */
export function FlipNumber({ value, className = '' }: { value: number | string; className?: string }) {
  const reduceMotion = useReducedMotion();
  const text = String(value);
  if (reduceMotion) return <span className={`tabular-nums ${className}`}>{text}</span>;
  return <span aria-label={text} className={`inline-flex tabular-nums ${className}`} data-testid="flip-number">
    {Array.from(text).map((digit, index) => (
      <span key={text.length - index} aria-hidden="true" className="relative inline-block overflow-hidden align-bottom" style={{ width: /\d/.test(digit) ? '0.65em' : '0.4em', height: '1.3em', perspective: 120 }}>
        <AnimatePresence initial={false} mode="popLayout">
          <motion.span key={digit} className="inline-block w-full text-center" initial={{ rotateX: -80, y: '35%', opacity: 0 }} animate={{ rotateX: 0, y: 0, opacity: 1 }} exit={{ rotateX: 80, y: '-35%', opacity: 0 }} transition={{ duration: 0.18, ease: [0.2, 0, 0, 1] }} style={{ backfaceVisibility: 'hidden' }}>{digit}</motion.span>
        </AnimatePresence>
      </span>
    ))}
  </span>;
}

export function FlipStatusText({ text }: { text: string }) {
  return <span aria-label={text}><span aria-hidden="true">{text.split(/(\d+(?:[.,]\d+)?)/u).map((part, index) => /^\d/u.test(part) ? <FlipNumber key={index} value={part} /> : <span key={index}>{part}</span>)}</span></span>;
}
