import { expect, test } from '@playwright/test';

test('computer workflow keeps one compact summary and inspectable live action receipts', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 700 });
  await page.goto('/e2e/fixtures/computer-use-trace.html');
  const group = page.getByTestId('computer-use-trace');
  const summary = group.locator('summary');
  await expect(summary).toContainText('Computer Use');
  await expect(summary).toContainText('3 steps');
  await expect(summary).toHaveAttribute('aria-busy', 'true');
  await summary.click();
  await expect(group.locator('.chat-tool-card')).toHaveCount(3);
  await page.evaluate(() => (window as unknown as { __COMPUTER_TRACE_TEST__: { fail(): void } }).__COMPUTER_TRACE_TEST__.fail());
  await expect(summary).toHaveAttribute('aria-busy', 'false');
  await expect(summary).toContainText('error');
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('computer-use-trace.png') });
});
