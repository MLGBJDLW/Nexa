import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('nexa-locale', 'en'));
  await page.goto('/e2e/fixtures/chat-image-preview.html');
});

for (const name of ['Markdown screenshot', 'Local screenshot', 'Uploaded screenshot.png', 'Generated screenshot']) {
  test(`${name} opens a keyboard-accessible image viewer`, async ({ page }) => {
    const thumbnail = page.getByRole('img', { name, exact: true });
    await expect(thumbnail).toBeVisible();
    await thumbnail.click();
    const viewer = page.getByTestId('image-lightbox');
    await expect(viewer).toBeVisible();
    await expect(viewer.getByRole('img')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(viewer).toHaveCount(0);
    await expect(thumbnail.locator('..')).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(viewer).toBeVisible();
  });
}

test('desktop screenshot survives durable result reconciliation and opens at full size', async ({ page }) => {
  const evidence = page.getByTestId('tool-visual-evidence');
  await expect(evidence).toBeVisible();
  await page.getByRole('button', { name: 'Reconcile durable tool result' }).click();
  await expect(evidence).toBeVisible();
  await evidence.getByRole('img').click();
  await expect(page.getByTestId('image-lightbox')).toBeVisible();
});

for (const mode of ['compact', 'trace']) test(`${mode} tool card displays captured pixels`, async ({ page }) => {
  await page.goto(`/e2e/fixtures/chat-image-preview.html?${mode}=1`);
  const evidence = page.getByTestId('tool-visual-evidence');
  await expect(evidence).toBeVisible();
  await evidence.getByRole('img').click();
  await expect(page.getByTestId('image-lightbox')).toBeVisible();
});

test('Done keeps a screenshot visible through saved-message hydration without persisting pixels across reload', async ({ page }) => {
  await page.goto('/e2e/fixtures/chat-image-preview.html?session=1');
  await expect(page.getByTestId('session-status')).toHaveText('settled / loaded');
  await page.getByRole('button', { name: 'Capture screenshot', exact: true }).click();
  await expect(page.getByTestId('session-status')).toHaveText('streaming / loaded');
  await expect(page.getByTestId('tool-visual-evidence')).toBeVisible();
  await page.getByRole('button', { name: 'Finish turn' }).click();
  await expect(page.getByTestId('session-status')).toHaveText('settled / loaded');
  await expect(page.getByTestId('tool-visual-evidence')).toBeVisible();
  await page.getByRole('button', { name: 'Refresh stored messages' }).click();
  await expect(page.getByTestId('tool-visual-evidence')).toBeVisible();
  await page.reload();
  await expect(page.getByTestId('session-status')).toHaveText('settled / loaded');
  await expect(page.getByTestId('tool-visual-evidence')).toHaveCount(0);
  await expect(page.getByTestId('tool-call-card')).toBeVisible();
});

test('image viewer fits narrow windows and zooms without losing its close control', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 650 });
  await page.getByRole('img', { name: 'Markdown screenshot', exact: true }).click();
  const viewer = page.getByTestId('image-lightbox');
  const picture = viewer.getByRole('img');
  const fit = await picture.boundingBox();
  await viewer.getByRole('button', { name: 'Zoom in', exact: true }).click();
  await expect.poll(async () => (await picture.boundingBox())!.width).toBeGreaterThan(fit!.width);
  await expect(viewer.getByRole('button', { name: 'Close', exact: true })).toBeInViewport();
  await viewer.getByRole('button', { name: 'Reset zoom' }).click();
  await page.setViewportSize({ width: 320, height: 560 });
  await expect(picture).toBeInViewport();
  await page.screenshot({ path: testInfo.outputPath('image-lightbox-narrow.png') });
  await viewer.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(viewer).toHaveCount(0);
});

for (const mode of ['', 'compact', 'trace', 'portrait']) {
  test(`image frames fit displayed pixels across window sizes (${mode || 'standard'})`, async ({ page }, testInfo) => {
    await page.goto(`/e2e/fixtures/chat-image-preview.html?${mode}=1`);
    for (const width of [1100, 390]) {
      await page.setViewportSize({ width, height: 720 });
      const frames = page.getByTestId('tool-image-frame');
      await expect(frames).toHaveCount(2);
      for (const frame of await frames.all()) {
        const picture = frame.getByRole('img');
        await expect(picture).toBeVisible();
        await expect.poll(async () => picture.evaluate(img => (img as HTMLImageElement).naturalWidth)).toBeGreaterThan(0);
        const bounds = await frame.boundingBox();
        const pixels = await picture.boundingBox();
        expect(Math.abs(bounds!.width - pixels!.width)).toBeLessThanOrEqual(3);
        expect(Math.abs(bounds!.height - pixels!.height)).toBeLessThanOrEqual(3);
        expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
        const ratio = await picture.evaluate(img => (img as HTMLImageElement).naturalWidth / (img as HTMLImageElement).naturalHeight);
        expect(Math.abs(pixels!.width / pixels!.height - ratio)).toBeLessThan(0.02);
      }
    }
    await page.getByTestId('tool-visual-evidence').scrollIntoViewIfNeeded();
    await page.screenshot({ path: testInfo.outputPath(`image-frame-${mode || 'standard'}.png`) });
  });
}
