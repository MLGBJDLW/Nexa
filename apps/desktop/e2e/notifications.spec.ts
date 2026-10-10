import { expect, test } from '@playwright/test';

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
