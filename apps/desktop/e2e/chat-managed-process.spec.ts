import { expect, test } from '@playwright/test';

test('server progress stays visible and its verified URL opens the browser workspace', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale', 'en');
    window.addEventListener('nexa:open-browser-workspace', (event) => {
      event.preventDefault();
      (window as Window & { openedUrl?: string }).openedUrl = (event as CustomEvent).detail.url;
    });
  });
  await page.goto('/e2e/fixtures/managed-process.html');
  const card = page.getByTestId('managed-process-card');
  await expect(card).toBeVisible();
  await expect(card).toHaveAttribute('data-process-state', 'running');
  await expect(card.getByTestId('managed-process-output')).toContainText('Starting development server...');
  await expect(card.getByTestId('managed-process-output')).toContainText('Compiling components...');
  await expect(card.getByRole('button')).toHaveCount(0);
  await page.getByRole('button', { name: 'Report ready' }).click();
  await expect(card).toHaveAttribute('data-process-state', 'ready');
  await expect(card).toContainText('Last reported');
  await card.getByRole('button', { name: /Open in browser/ }).click();
  await expect.poll(() => page.evaluate(() => (window as Window & { openedUrl?: string }).openedUrl)).toBe('http://127.0.0.1:5173/');
  await page.setViewportSize({ width: 360, height: 740 });
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});
