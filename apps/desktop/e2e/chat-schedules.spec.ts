import { test, expect } from '@playwright/test';

test('sidebar and capsule share scheduled task state and preserve the paused indicator', async ({ page }, testInfo) => {
  await page.addInitScript(() => localStorage.setItem('nexa-locale', 'en'));
  await page.goto('/e2e/fixtures/chat-schedules.html');
  const badges = page.getByTestId('conversation-schedule-badge');
  await expect(badges).toHaveCount(2);
  await expect(badges.first()).toHaveAttribute('title', /Morning research.*08:00.*Asia\/Shanghai/);
  await page.getByTestId('task-board-collapsed').click();
  const schedules = page.getByTestId('conversation-schedules');
  await expect(schedules).toContainText('Morning research');
  await expect(schedules).toContainText('Asia/Shanghai');
  await page.evaluate(() => (window as any).holdNextScheduleRead());
  await page.waitForFunction(() => typeof (window as any).releaseScheduleRead === 'function');
  await schedules.getByRole('button', { name: 'Pause task: Morning research' }).click();
  await page.evaluate(() => (window as any).releaseScheduleRead());
  await expect(schedules).toContainText('Paused');
  await expect(badges.first()).toHaveAttribute('title', /Paused/);
  await page.screenshot({ path: testInfo.outputPath('schedule-capsule-paused.png') });
  await schedules.getByRole('button', { name: 'Resume task: Morning research' }).click();
  await expect(schedules).toContainText('Next:');
  await expect(schedules.getByRole('link', { name: 'Manage' })).toHaveAttribute('href', '/workflows?tab=automations');
});
