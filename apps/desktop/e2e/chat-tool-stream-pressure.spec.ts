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
    for (const toggle of await page.getByTestId('thinking-trace-toggle').all()) {
      if (await toggle.getAttribute('aria-expanded') !== 'true') await toggle.click();
    }
    const before = page.getByText('Before tool reply.', { exact: true });
    const continuation = page.getByText(/Answer 10\./);
    await expect(before).toHaveCount(1);
    await expect(continuation).toHaveCount(1);
    await expect(continuation).toBeVisible({ timeout: 2500 });
    await expect(page.getByText(/Thought 10\./)).toBeVisible();
    const tool = page.getByTestId('tool-call-card').filter({ hasText: 'Browser frame' });
    const interaction = page.getByTestId('interaction-trace').filter({ has: tool });
    await expect(interaction.locator('summary')).toHaveAttribute('aria-busy', 'true');
    await interaction.locator('summary').click();
    await expect(tool).toBeVisible();
    const order = await before.evaluate((node) => {
      const card = [...document.querySelectorAll('[data-testid="tool-call-card"]')].find(card => card.textContent?.includes('Browser frame'));
      const reply = [...document.querySelectorAll('p')].find(paragraph => paragraph.textContent?.includes('Answer 10.'));
      return !!card && !!reply
        && !!(node.compareDocumentPosition(card) & Node.DOCUMENT_POSITION_FOLLOWING)
        && !!(card.compareDocumentPosition(reply) & Node.DOCUMENT_POSITION_FOLLOWING);
    });
    expect(order, 'before-text, active tool and continued reply retain chronological DOM order').toBe(true);
    await expect(page.getByTestId('pressure-running')).toHaveText('true');
  } finally {
    await page.evaluate(() => (window as any).__stopToolPressure?.());
  }
});
