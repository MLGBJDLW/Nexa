import { createTerminalInputWriter } from '../src/lib/terminalInput';

function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

async function run() {
  let release!: () => void;
  const blocked = new Promise<void>(resolve => { release = resolve; });
  const calls: string[] = [];
  const write = createTerminalInputWriter(async (id, data) => {
    calls.push(`${id}:${data}`);
    if (data === 'first') await blocked;
  });
  const first = write('one', 'first');
  const second = write('one', 'second');
  const enter = write('one', '\r');
  await write('two', 'independent');
  assert(calls.join('|') === 'one:first|two:independent', 'blocked input must preserve order without blocking other sessions');
  release();
  await Promise.all([first, second, enter]);
  assert(calls.join('|') === 'one:first|two:independent|one:second|one:\r', 'rapid keystrokes must reach the native pipe in order');

  let attempts = 0;
  const failing = createTerminalInputWriter(async () => {
    attempts += 1;
    if (attempts === 1) throw new Error('pipe closed');
  });
  const failed = await Promise.allSettled([failing('one', 'partial'), failing('one', '\r')]);
  assert(failed.every(result => result.status === 'rejected') && attempts === 1, 'never submit the remainder of a partially failed command');
  await failing('one', 'new explicit input');
  assert(Number(attempts) === 2, 'settled failures must release queue state');

  let unblock!: () => void;
  const capacity = createTerminalInputWriter(() => new Promise<void>(resolve => { unblock = resolve; }));
  const full = capacity('one', 'x'.repeat(256 * 1024));
  const overflow = await capacity('one', 'x').then(() => false, () => true);
  assert(overflow, 'blocked native input must not retain unbounded queued data');
  unblock();
  await full;
  console.log('ok - terminal input preserves order, isolates sessions, bounds backlog, and stops after failure');
}

void run().catch(error => { console.error(error); throw error; });
