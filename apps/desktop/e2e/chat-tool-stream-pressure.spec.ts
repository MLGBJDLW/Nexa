import { expect, test } from './timeline-test';

test('thoughts and replies commit while a long tool continues publishing progress', async ({ page }) => {
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/e2e/fixtures/stream-tool-pressure.tsx';
    (await import(/* @vite-ignore */ path)).renderStreamToolPressure();
  });
  await page.getByRole('button', { name: 'Start long browser tool' }).click();
  try {
    await expect.poll(() => page.evaluate(() => Number(document.documentElement.dataset.pressureEvents))).toBeGreaterThan(15);
    await expect(page.getByTestId('pressure-answer')).toContainText('Answer 10.', { timeout: 2500 });
    await expect(page.getByTestId('pressure-thinking')).toContainText('Thought 10.', { timeout: 2500 });
    await expect(page.getByTestId('pressure-running')).toHaveText('true');
  } finally {
    await page.evaluate(() => (window as any).__stopToolPressure?.());
  }
});
