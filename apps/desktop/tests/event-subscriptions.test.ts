import { connectEventSubscriptions, type EventConnectionState } from '../src/lib/eventSubscriptions';

function check(condition: boolean, message: string): void { if (!condition) throw new Error(message); }

async function run() {
  let attempts = 0;
  let disposed = 0;
  const states: string[] = [];
  let ready!: () => void;
  const settled = new Promise<void>(resolve => { ready = resolve; });
  const connection = connectEventSubscriptions([
    async () => () => { disposed++; },
    async () => { if (attempts++ === 0) throw new Error('transient'); return () => { disposed++; }; },
  ], ({ status }) => { states.push(status); if (status === 'ready') ready(); }, { retryDelaysMs: [1] });
  await settled;
  check(attempts === 2, 'A failed listener must be retried');
  check(disposed === 1, 'Partial success must be disposed before reconnecting');
  connection.stop();
  connection.stop();
  check(disposed === 3, 'Every successful registration is disposed exactly once');
  check(states.join(',') === 'connecting,connecting,ready', 'The group is ready only after all listeners connect');

  let release!: (unlisten: () => void) => void;
  let failed!: () => void;
  const failure = new Promise<void>(resolve => { failed = resolve; });
  let lateDisposed = 0;
  let active: (() => boolean) | undefined;
  const late = connectEventSubscriptions([
    isActive => { active = isActive; return new Promise<() => void>(resolve => { release = resolve; }); },
  ], (state: EventConnectionState) => { if (state.status === 'error') failed(); }, { timeoutMs: 5, retryDelaysMs: [] });
  await failure;
  check(active?.() === false, 'A timed-out native listener must not deliver events');
  release(() => { lateDisposed++; });
  await Promise.resolve();
  await Promise.resolve();
  check(lateDisposed === 1, 'A listener arriving after timeout must be disposed');
  late.stop();
  console.log('ok - event subscriptions recover partial failure without leaked or stale listeners');
}
void run().catch(error => { console.error(error); throw error; });
