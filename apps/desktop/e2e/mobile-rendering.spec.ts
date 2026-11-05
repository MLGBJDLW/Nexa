import { test, expect } from '@playwright/test';

for (const width of [320, 360, 412, 720]) test(`mobile markdown and charts fit ${width}px`, async ({page}, info) => {
  await page.setViewportSize({width,height:820});
  await page.goto('/e2e/fixtures/mobile-rendering.html');
  await expect(page.getByTestId('generative-ui')).toBeVisible();
  await expect(page.getByTestId('mermaid-surface').locator('svg')).toBeVisible();
  expect(await page.locator('.native-chat-content').evaluate(node=>node.scrollWidth-node.clientWidth)).toBeLessThanOrEqual(1);
  expect(await page.locator('.nexa-ui').evaluate(node=>node.scrollWidth-node.clientWidth)).toBeLessThanOrEqual(1);
  const bars = page.locator('.nexa-ui-bar-track');
  expect((await bars.first().boundingBox())!.width).toBeGreaterThan(100);
  const frame=page.getByTestId('html-preview').locator('iframe').contentFrame();
  await expect(frame.getByRole('heading',{name:'HTML 图表'})).toBeVisible();
  expect(await frame.locator('body').evaluate(node=>node.scrollWidth-node.clientWidth)).toBeLessThanOrEqual(1);
  await page.getByTestId('generative-ui').screenshot({path:info.outputPath(`mobile-chart-${width}.png`)});
});
