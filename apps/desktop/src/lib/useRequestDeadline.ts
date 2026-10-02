import { useEffect, useState } from 'react';
import { appTimeMs } from './dateTime';

/** Display clocks do not resolve, extend or otherwise mutate a backend request. */
export function useRequestDeadline(expiresAt?: string | null) {
  const deadline = expiresAt ? appTimeMs(expiresAt) : Number.NaN;
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    setNow(Date.now());
    if (!Number.isFinite(deadline) || deadline <= Date.now()) return;
    const timer = window.setInterval(() => {
      const time = Date.now();
      setNow(time);
      if (time >= deadline) window.clearInterval(timer);
    }, 1_000);
    return () => window.clearInterval(timer);
  }, [deadline]);
  return Number.isFinite(deadline) ? Math.max(0, Math.ceil((deadline - now) / 1_000)) : null;
}
