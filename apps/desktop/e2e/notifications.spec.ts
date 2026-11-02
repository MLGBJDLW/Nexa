import { expect, test } from '@playwright/test';

test('short notifications remain one compact row when dynamic library styles are unavailable', async ({ page }, testInfo) => {
  await page.goto('/e2e/fixtures/notifications.html');
  await page.evaluate(() => {
    for (const style of document.querySelectorAll('style:not([data-vite-dev-id])')) {
      if (style.textContent?.includes('[data-sonner-toaster]')) style.remove();
    }
  });
  await page.getByRole('button', { name: 'Show success', exact: true }).click();
  const toast = page.locator('[data-sonner-toast]').first();
  await expect(toast).toContainText('提供商已保存');
  await expect(toast).toHaveAttribute('data-mounted', 'true');
  await expect(toast).toHaveCSS('opacity', '1');
  const box = (await toast.boundingBox())!;
  const icon = (await toast.locator('[data-icon]').boundingBox())!;
  const title = (await toast.locator('[data-title]').boundingBox())!;
  const close = toast.getByRole('button', { name: 'Close', exact: true });
  const dismiss = (await close.boundingBox())!;
  expect(box.height).toBeLessThanOrEqual(48);
  expect(Math.abs(icon.y + icon.height / 2 - title.y - title.height / 2)).toBeLessThan(3);
  expect(icon.x + icon.width).toBeLessThan(title.x);
  expect(dismiss.x).toBeGreaterThan(title.x + title.width);
  expect(dismiss.x + dismiss.width).toBeLessThanOrEqual(box.x + box.width);
  await page.screenshot({ path: testInfo.outputPath('compact-notification.png') });
  await close.click();
  await expect(toast).toHaveCount(0);
});

test('notifications have opaque themed surfaces, readable content and a reachable dismiss action', async ({ page }, testInfo) => {
  for (const theme of ['dark','light','dream']) {
    await page.goto(`/e2e/fixtures/notifications.html?theme=${theme}`);
    await page.getByRole('button', {name:'Show warning',exact:true}).click();
    const toast=page.locator('[data-sonner-toast]').first();
    await expect(toast).toContainText('Needs attention');
    await expect(toast).toHaveAttribute('data-mounted', 'true');
    await expect(toast).toHaveCSS('opacity', '1');
    await expect(toast).toHaveCSS('background-color', theme==='light'?'rgb(255, 255, 255)':'rgb(25, 27, 34)');
    await expect(toast).not.toHaveCSS('color','rgba(0, 0, 0, 0)');
    const box=await toast.boundingBox();
    const composer=await page.getByRole('textbox',{name:'Composer'}).boundingBox();
    expect(box!.y+box!.height).toBeLessThan(composer!.y);
    await page.screenshot({path:testInfo.outputPath(`notifications-${theme}.png`), animations:'disabled'});
    await toast.getByRole('button',{name:'Close',exact:true}).click();
    await expect(toast).toHaveCount(0);
  }
});

test('long stacked notifications stay within a small viewport and leave the composer usable', async ({page})=>{
  await page.setViewportSize({width:390,height:720});
  await page.goto('/e2e/fixtures/notifications.html');
  await page.getByRole('button',{name:'Show many',exact:true}).click();
  const front=page.locator('[data-sonner-toast][data-front="true"]');
  await expect(front).toBeVisible();
  const box=await front.boundingBox();
  expect(box!.height).toBeLessThanOrEqual(202);
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.x+box!.width).toBeLessThanOrEqual(391);
  await page.getByRole('textbox',{name:'Composer'}).fill('Still able to compose');
  await expect(page.getByRole('textbox',{name:'Composer'})).toHaveValue('Still able to compose');
});
