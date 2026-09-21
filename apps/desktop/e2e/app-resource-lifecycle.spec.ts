import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.clock.install();
  await page.addInitScript(() => {
    localStorage.clear();
    localStorage.setItem('nexa-theme', 'dark');
    let registry = { version: 2, initialized: true, revision: 1, activeThemeId: 'dark', plugins: [] };
    const metrics = { reads: 0, active: 0, peak: 0, bytes: 0, delay: 0, hydrate: 0, failures: 0, hang: false };
    const blockedReads: Array<() => void> = [];
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, { event: string; handler: number }>();
    let next = 1;
    let hidden = false;
    Object.defineProperty(document, 'hidden', { get: () => hidden });
    const emit = (event: string, payload: unknown) => {
      for (const [id, listener] of listeners) {
        if (listener.event === event) callbacks.get(listener.handler)?.({ event, id, payload });
      }
    };
    Object.assign(window, {
      __appearanceMetrics: metrics,
      __setHidden(value: boolean) { hidden = value; document.dispatchEvent(new Event('visibilitychange')); },
      __changeAppearance(activeThemeId: string, emitEvent = true) {
        registry = { ...registry, activeThemeId, revision: registry.revision + 1 };
        if (emitEvent) emit('appearance://changed', registry);
      },
      __invalidateAppearance() { emit('appearance://changed', { revision: registry.revision }); },
      __releaseAppearanceReads() { metrics.hang = false; blockedReads.splice(0).forEach(resolve => resolve()); },
      __appearanceListenerCount: () => listeners.size,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: (_event: string, id: number) => listeners.delete(id) },
      __TAURI_INTERNALS__: {
        transformCallback(callback: (event: unknown) => void) { const id = next++; callbacks.set(id, callback); return id; },
        unregisterCallback(id: number) { callbacks.delete(id); },
        async invoke(cmd: string, args: Record<string, unknown> = {}) {
          if (cmd === 'plugin:event|listen') { const id = next++; listeners.set(id, { event: String(args.event), handler: Number(args.handler) }); return id; }
          if (cmd === 'plugin:event|unlisten') { listeners.delete(Number(args.eventId)); return; }
          if (cmd === 'hydrate_appearance_registry_cmd') { metrics.hydrate++; return registry; }
          if (cmd === 'get_appearance_registry_cmd') {
            metrics.reads++;
            metrics.active++;
            metrics.peak = Math.max(metrics.peak, metrics.active);
            const snapshot = structuredClone(registry);
            if (metrics.failures > 0) { metrics.failures--; metrics.active--; throw new Error('simulated registry read failure'); }
            if (metrics.hang) await new Promise<void>(resolve => blockedReads.push(resolve));
            if (metrics.delay) await new Promise(resolve => setTimeout(resolve, metrics.delay));
            metrics.active--;
            metrics.bytes += JSON.stringify(snapshot).length;
            return snapshot;
          }
          return null;
        },
      },
    });
  });
  await page.goto('/e2e/fixtures/app-resource-lifecycle.html');
  await expect(page.getByTestId('active-appearance')).toHaveText('dark');
  await expect.poll(() => page.evaluate(() => (window as any).__appearanceMetrics.hydrate)).toBe(1);
});

test('appearance idle and hidden work is bounded and native changes apply immediately', async ({ page }) => {
  await page.clock.runFor(60_000);
  const idle = await page.evaluate(() => (window as any).__appearanceMetrics);
  console.log('appearance idle 60s:', JSON.stringify(idle));
  expect(idle.reads).toBeLessThanOrEqual(2);
  await page.evaluate(() => (window as any).__setHidden(true));
  await page.clock.runFor(60_000);
  expect(await page.evaluate(() => (window as any).__appearanceMetrics.reads)).toBe(idle.reads);
  await page.evaluate(() => (window as any).__changeAppearance('light'));
  await expect(page.getByTestId('active-appearance')).toHaveText('light');
  await page.evaluate(() => { (window as any).__changeAppearance('dream', false); (window as any).__setHidden(false); });
  await expect(page.getByTestId('active-appearance')).toHaveText('dream');
  await page.evaluate(() => (window as any).unmountAppearance());
  expect(await page.evaluate(() => (window as any).__appearanceListenerCount())).toBe(0);
  const stopped = await page.evaluate(() => (window as any).__appearanceMetrics.reads);
  await page.clock.runFor(60_000);
  expect(await page.evaluate(() => (window as any).__appearanceMetrics.reads)).toBe(stopped);
});

test('slow appearance reads never overlap or replace a newer native revision', async ({ page }) => {
  await page.evaluate(() => { (window as any).__appearanceMetrics.delay = 45_000; });
  await page.clock.runFor(60_000);
  const slow = await page.evaluate(() => (window as any).__appearanceMetrics);
  console.log('appearance slow 60s:', JSON.stringify(slow));
  expect(slow.peak).toBe(1);
  await page.evaluate(() => (window as any).__changeAppearance('light'));
  await expect(page.getByTestId('active-appearance')).toHaveText('light');
  await page.clock.runFor(45_000);
  await expect(page.getByTestId('active-appearance')).toHaveText('light');
});

test('agent appearance invalidation reconciles after an older read completes', async ({ page }) => {
  await page.evaluate(() => { (window as any).__appearanceMetrics.delay = 2_000; });
  await page.clock.runFor(30_000);
  await page.evaluate(() => {
    (window as any).__changeAppearance('bloom', false);
    (window as any).__invalidateAppearance();
  });
  await page.clock.runFor(4_001);
  await expect(page.getByTestId('active-appearance')).toHaveText('bloom');
  expect(await page.evaluate(() => (window as any).__appearanceMetrics.peak)).toBe(1);
  expect(await page.evaluate(() => (window as any).__appearanceMetrics.reads)).toBe(2);
});

test('appearance reads recover from rejection and remain bounded while a native reply is stalled', async ({ page }) => {
  await page.evaluate(() => { (window as any).__appearanceMetrics.failures = 1; });
  await page.clock.runFor(30_000);
  expect(await page.evaluate(() => (window as any).__appearanceMetrics.active)).toBe(0);
  await page.evaluate(() => (window as any).__changeAppearance('light', false));
  await page.clock.runFor(30_000);
  await expect(page.getByTestId('active-appearance')).toHaveText('light');
  await page.evaluate(() => { (window as any).__appearanceMetrics.hang = true; });
  await page.clock.runFor(30_000);
  const stalledReads = await page.evaluate(() => (window as any).__appearanceMetrics.reads);
  await page.evaluate(() => {
    (window as any).__changeAppearance('bloom', false);
    (window as any).__invalidateAppearance();
    for (let i = 0; i < 10; i++) { (window as any).__setHidden(true); (window as any).__setHidden(false); }
  });
  await page.clock.runFor(300_000);
  expect(await page.evaluate(() => (window as any).__appearanceMetrics.reads)).toBe(stalledReads);
  expect(await page.evaluate(() => (window as any).__appearanceMetrics.peak)).toBe(1);
  // Full authoritative change events can update appearance even if IPC replies
  // are stalled. Revision-only invalidations wait for the pending native read.
  await expect(page.getByTestId('active-appearance')).toHaveText('light');
  await page.evaluate(() => (window as any).__changeAppearance('dream'));
  await expect(page.getByTestId('active-appearance')).toHaveText('dream');
  await page.evaluate(() => (window as any).__releaseAppearanceReads());
  await expect.poll(() => page.evaluate(() => (window as any).__appearanceMetrics.reads)).toBe(stalledReads + 1);
  await expect(page.getByTestId('active-appearance')).toHaveText('dream');
});

test('unmount disposes listeners before a delayed native reply settles', async ({ page }) => {
  await page.evaluate(() => { (window as any).__appearanceMetrics.hang = true; });
  await page.clock.runFor(30_000);
  await page.evaluate(() => { (window as any).__invalidateAppearance(); (window as any).unmountAppearance(); });
  await expect(page.getByTestId('active-appearance')).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__appearanceListenerCount())).toBe(0);
  const stopped = await page.evaluate(() => (window as any).__appearanceMetrics.reads);
  await page.evaluate(() => (window as any).__releaseAppearanceReads());
  await page.clock.runFor(60_000);
  expect(await page.evaluate(() => (window as any).__appearanceMetrics.reads)).toBe(stopped);
  expect(await page.evaluate(() => (window as any).__appearanceMetrics.active)).toBe(0);
});
