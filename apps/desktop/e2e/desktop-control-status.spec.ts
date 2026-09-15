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
    Object.assign(window, { __desktopStops: stopped, __TAURI_INTERNALS__: {
      transformCallback(callback: (event: unknown) => void) { const id = seq++; callbacks.set(id, callback); return id; },
      unregisterCallback(id: number) { callbacks.delete(id); },
      async invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'plugin:event|listen') { const id = seq++; listeners.set(id, { event: String(args.event), handler: Number(args.handler) }); return id; }
        if (command === 'plugin:event|unlisten') { listeners.delete(Number(args.eventId)); return null; }
        if (command === 'desktop_control_status_cmd') return active;
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
  await page.screenshot({ path: testInfo.outputPath('desktop-status.png') });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect.poll(() => page.evaluate(() => document.getAnimations().filter(animation => animation.playState === 'running').length)).toBe(0);
  await page.getByRole('button', { name: 'Stop', exact: true }).click();
  expect(await page.evaluate(() => (window as unknown as { __desktopStops: string[] }).__desktopStops)).toEqual(['desktop-task']);
  await expect(page.getByTestId('computer-use-status')).toHaveCount(0);
});
