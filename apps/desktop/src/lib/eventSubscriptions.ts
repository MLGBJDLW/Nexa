export type EventSubscription = (isActive: () => boolean) => Promise<() => void>;
export type EventConnectionState = { status: 'connecting' | 'ready' | 'error'; error?: unknown };

/** Own a complete listener set, including partial failures and late native replies. */
export function connectEventSubscriptions(
  subscriptions: EventSubscription[],
  onState: (state: EventConnectionState) => void,
  { timeoutMs = 10_000, retryDelaysMs = [250, 1000] }: { timeoutMs?: number; retryDelaysMs?: number[] } = {},
): { stop: () => void; retry: () => void } {
  let stopped = false;
  let failures = 0;
  let disposeAttempt: (() => void) | undefined;
  let retryTimer: ReturnType<typeof setTimeout> | undefined;

  const disconnect = () => {
    clearTimeout(retryTimer);
    disposeAttempt?.();
  };
  const start = () => {
    if (stopped) return;
    disconnect();
    let active = true;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const unlisteners: Array<() => void> = [];
    const unlisten = (callback: () => void) => {
      try { callback(); } catch { /* A failed disposal must not retain the other listeners. */ }
    };
    disposeAttempt = () => {
      active = false;
      clearTimeout(timeout);
      unlisteners.splice(0).forEach(unlisten);
    };
    onState({ status: 'connecting' });
    const ready = Promise.all(subscriptions.map(async (subscribe) => {
      const callback = await subscribe(() => active && !stopped);
      if (active && !stopped) unlisteners.push(callback);
      else unlisten(callback);
    }));
    const deadline = new Promise<never>((_, reject) => {
      timeout = setTimeout(() => reject(new Error('Native event connection timed out')), timeoutMs);
    });
    void Promise.race([ready, deadline]).then(() => {
      if (!active || stopped) return;
      clearTimeout(timeout);
      onState({ status: 'ready' });
    }).catch((error: unknown) => {
      if (!active || stopped) return;
      disposeAttempt?.();
      const delay = retryDelaysMs[failures++];
      if (delay === undefined) onState({ status: 'error', error });
      else retryTimer = setTimeout(start, delay);
    });
  };
  start();
  return {
    stop() { stopped = true; disconnect(); },
    retry() { if (!stopped) { failures = 0; start(); } },
  };
}
