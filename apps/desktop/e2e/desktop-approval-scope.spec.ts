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

test('native agent questions show exact choices without reusable or default approval', async ({ page }, testInfo) => {
  await page.addInitScript(() => { localStorage.setItem('nexa-locale', 'en'); });
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/e2e/fixtures/approval-dialog.tsx';
    (await import(/* @vite-ignore */ path)).renderApproval('external_agent_choice');
  });
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('button', { name: 'First answer', exact: true })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Second answer', exact: true })).toBeVisible();
  await expect(dialog.getByRole('button', { name: /allow|session|advanced/i })).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath('native-agent-choices.png') });
});
