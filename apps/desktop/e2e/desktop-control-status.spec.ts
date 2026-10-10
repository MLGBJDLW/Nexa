import { expect, test } from '@playwright/test';

test('desktop computer-use status identifies active control and stops its owning task', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 350, height: 76 });
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale', 'en');
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, { event: string; handler: number }>();
    let seq = 1;
    const active = [{ conversationId: 'desktop-task', runId: 'desktop-run', callId: 'control', toolName: 'computer_control' }];
    const stopped: string[] = [];
    const appearances: Record<string, unknown>[] = [];
    let registry: Record<string, unknown> = { version: 2, revision: 1, initialized: true, activeThemeId: 'dark', plugins: [] };
    Object.assign(window, { __desktopStops: stopped, __desktopAppearances: appearances, __desktopTheme(accent: string) {
      registry = { version: 2, revision: Number(registry.revision) + 1, initialized: true, activeThemeId: 'control-theme', plugins: [{
        manifestVersion: 2, kind: 'theme-resource', id: 'control-theme', name: 'Control theme',
        theme: { baseTheme: 'dark', mode: 'dark', colors: { accent }, effects: {}, typography: {}, motion: {}, brand: {}, content: {}, components: {}, background: { kind: 'none' } },
      }] };
      for (const [id, listener] of listeners) if (listener.event === 'appearance://changed') callbacks.get(listener.handler)?.({ event: listener.event, id, payload: registry });
    }, __desktopWait() {
      for (const [id, listener] of listeners) if (listener.event === 'desktop-control:status') callbacks.get(listener.handler)?.({ event: listener.event, id, payload: active.map(activity => ({ ...activity, phase: 'waiting' })) });
    }, __TAURI_INTERNALS__: {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
      transformCallback(callback: (event: unknown) => void) { const id = seq++; callbacks.set(id, callback); return id; },
      unregisterCallback(id: number) { callbacks.delete(id); },
      async invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'plugin:event|listen') { const id = seq++; listeners.set(id, { event: String(args.event), handler: Number(args.handler) }); return id; }
        if (command === 'plugin:event|unlisten') { listeners.delete(Number(args.eventId)); return null; }
        if (command === 'desktop_control_status_cmd') return active;
        if (command === 'get_appearance_registry_cmd' || command === 'hydrate_appearance_registry_cmd') return registry;
        if (command === 'set_desktop_control_appearance_cmd') { appearances.push(args); return null; }
        if (command === 'stop_desktop_control_cmd') {
          stopped.push(String(args.conversationId));
          for (const [id, listener] of listeners) if (listener.event === 'desktop-control:status') callbacks.get(listener.handler)?.({ event: listener.event, id, payload: [] });
          return null;
        }
        return null;
      },
    } });
  });
  await page.goto('/desktop-control-status');
  await expect(page.getByRole('status')).toContainText('Nexa');
  await expect(page.getByRole('status')).toContainText('Controlling the computer');
  await expect(page.getByRole('button', { name: 'Stop', exact: true })).toBeInViewport();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __desktopAppearances: { accent: number[] }[] }).__desktopAppearances.at(-1)?.accent)).toEqual([20, 184, 166]);
  await page.evaluate(() => (window as unknown as { __desktopTheme(color: string): void }).__desktopTheme('#e06c32'));
  await expect.poll(() => page.evaluate(() => (window as unknown as { __desktopAppearances: { accent: number[] }[] }).__desktopAppearances.at(-1)?.accent)).toEqual([224, 108, 50]);
  await expect(page.locator('.nexa-control-bond')).toHaveCSS('color', 'rgb(224, 108, 50)');
  await page.screenshot({ path: testInfo.outputPath('desktop-status.png') });
  await page.evaluate(() => (window as unknown as { __desktopWait(): void }).__desktopWait());
  await expect(page.getByRole('status')).toContainText('Continuing the computer task');
  await expect(page.getByRole('button', { name: 'Stop', exact: true })).toBeVisible();
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect.poll(() => page.evaluate(() => (window as unknown as { __desktopAppearances: { reducedMotion: boolean }[] }).__desktopAppearances.at(-1)?.reducedMotion)).toBe(true);
  await expect.poll(() => page.evaluate(() => document.getAnimations().filter(animation => animation.playState === 'running').length)).toBe(0);
  await page.getByRole('button', { name: 'Stop', exact: true }).click();
  expect(await page.evaluate(() => (window as unknown as { __desktopStops: string[] }).__desktopStops)).toEqual(['desktop-task']);
  await expect(page.getByTestId('computer-use-status')).toHaveCount(0);
});
