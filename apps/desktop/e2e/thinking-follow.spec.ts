import { expect, test, type Page } from '@playwright/test';

// Native thumb dragging needs the scrollbar that Chromium headless hides by default.
test.use({ launchOptions: { ignoreDefaultArgs: ['--hide-scrollbars'] } });

async function renderTrace(page: Page, text: string, cardHeight = 0) {
  await page.evaluate(async ({ text, cardHeight }) => {
    const fixturePath = '/e2e/fixtures/thinking-follow.tsx';
    const { renderTrace } = await import(/* @vite-ignore */ fixturePath);
    renderTrace(text, cardHeight);
  }, { text, cardHeight });
}

const longThinking = Array.from({ length: 60 }, (_, i) => `Thinking line ${i}: inspect the next source.\n`).join('');

test('follows presented thinking and tool-only growth without new text', async ({ page }) => {
  await page.goto('/');
  await renderTrace(page, longThinking);
  await page.getByTestId('thinking-trace-toggle').click();
  const text = page.getByTestId('thinking-stream-content');
  const scroller = text.locator('xpath=ancestor::div[contains(@class,"overflow-y-auto")]');
  await expect(text).toHaveText(longThinking);
  await expect.poll(() => scroller.evaluate(el => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(3);
  await scroller.dispatchEvent('wheel', { deltaY: -120, ctrlKey: true });
  await renderTrace(page, longThinking, 500);
  await expect(page.getByTestId('trace-tool-card')).toBeVisible();
  await expect.poll(() => scroller.evaluate(el => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(3);
});

test('pauses for manual reading and resumes at the bottom', async ({ page }) => {
  await page.goto('/');
  await renderTrace(page, longThinking);
  await page.getByTestId('thinking-trace-toggle').click();
  const text = page.getByTestId('thinking-stream-content');
  const scroller = text.locator('xpath=ancestor::div[contains(@class,"overflow-y-auto")]');
  await expect(text).toHaveText(longThinking);
  await scroller.evaluate(el => { el.scrollTop = el.scrollHeight; });
  const thumb = await scroller.evaluate(el => {
    const rect = el.getBoundingClientRect();
    const height = Math.max(36, el.clientHeight * el.clientHeight / el.scrollHeight);
    return { x: rect.right - (el.offsetWidth - el.clientWidth) / 2, y: rect.bottom - height / 2 };
  });
  await page.mouse.move(thumb.x, thumb.y);
  await page.mouse.down();
  await page.mouse.move(thumb.x, thumb.y - 100, { steps: 10 });
  await page.mouse.up();
  await expect.poll(() => scroller.evaluate(el => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeGreaterThan(200);
  const originalTheme = await page.evaluate(() => {
    const previous = document.documentElement.className;
    document.documentElement.className = 'theme-dark';
    return previous;
  });
  await scroller.screenshot({ path: '.artifacts/thinking-scrollbar-dark.png' });
  await page.evaluate(() => { document.documentElement.className = 'theme-light'; });
  await scroller.screenshot({ path: '.artifacts/thinking-scrollbar-light.png' });
  await page.evaluate(previous => { document.documentElement.className = previous; }, originalTheme);
  const readingTop = await scroller.evaluate(el => el.scrollTop);
  await renderTrace(page, longThinking, 500);
  await expect(page.getByTestId('trace-tool-card')).toBeAttached();
  await expect.poll(() => scroller.evaluate(el => el.scrollTop)).toBe(readingTop);
  await scroller.evaluate(el => { el.scrollTop = el.scrollHeight; });
  await renderTrace(page, longThinking, 800);
  await expect.poll(() => scroller.evaluate(el => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(3);
});

test('streaming reasoning stays collapsed until requested and respects manual collapse', async ({ page }) => {
  await page.goto('/');
  await renderTrace(page, 'Private working notes');
  const toggle = page.getByTestId('thinking-trace-toggle');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(page.getByTestId('thinking-stream-content')).toHaveCount(0);
  await toggle.click();
  await expect(page.getByTestId('thinking-stream-content')).toContainText('Private working notes');
  await toggle.click();
  await renderTrace(page, 'Private working notes with a new token');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(page.getByTestId('thinking-stream-content')).toHaveCount(0);
});
