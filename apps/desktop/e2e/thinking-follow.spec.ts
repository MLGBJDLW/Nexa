import { expect, test, type Page } from '@playwright/test';

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
  const text = page.getByTestId('thinking-stream-content');
  const scroller = text.locator('xpath=ancestor::div[contains(@class,"overflow-y-auto")]');
  await expect(text).toHaveText(longThinking);
  await scroller.evaluate(el => { el.scrollTop = el.scrollHeight; });
  await scroller.hover();
  await page.mouse.wheel(0, -400);
  await expect.poll(() => scroller.evaluate(el => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeGreaterThan(200);
  const readingTop = await scroller.evaluate(el => el.scrollTop);
  await renderTrace(page, longThinking, 500);
  await expect(page.getByTestId('trace-tool-card')).toBeAttached();
  await expect.poll(() => scroller.evaluate(el => el.scrollTop)).toBe(readingTop);
  await scroller.evaluate(el => { el.scrollTop = el.scrollHeight; });
  await renderTrace(page, longThinking, 800);
  await expect.poll(() => scroller.evaluate(el => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(3);
});
