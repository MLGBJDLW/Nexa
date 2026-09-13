import { test, expect } from '@playwright/test';

test('only verified window-task requests offer a reusable desktop grant', async ({ page }) => {
  await page.addInitScript(() => { localStorage.setItem('nexa-locale', 'en'); });
  await page.goto('/');
  const render = (kind: string) => page.evaluate(async (kind) => {
    const path = '/e2e/fixtures/approval-dialog.tsx';
    (await import(/* @vite-ignore */ path)).renderApproval(kind);
  }, kind);
  await render('desktop_action');
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('button', { name: /session/i })).toHaveCount(0);
  await render('desktop_window_task');
  await expect(dialog.getByRole('button', { name: /session/i })).toBeVisible();
  await expect(dialog.getByText('verified-scope')).toHaveCount(0);
  await expect(dialog.getByRole('button', { name: /advanced/i })).toHaveCount(0);
});
