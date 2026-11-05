import { expect, test } from '@playwright/test';

test('storage management measures categories and archives or deletes an explicit selection across pages', async ({page}, info) => {
  await page.addInitScript(() => localStorage.setItem('nexa-locale','en'));
  await page.goto('/e2e/fixtures/storage-settings.html');
  await expect(page.getByRole('img',{name:'Storage by category'})).toBeVisible();
  await expect(page.getByText('Local models',{exact:true})).toBeVisible();
  await page.getByRole('button',{name:'Manage conversations',exact:true}).click();
  await page.getByRole('button',{name:'Select All',exact:true}).click();
  await expect(page.getByText('101 selected',{exact:true})).toBeVisible();
  await page.getByRole('button',{name:'Archive',exact:true}).click();
  await expect.poll(() => page.evaluate(() => (window as any).storageMutations[0]?.ids.length)).toBe(101);
  await page.getByLabel('Conversation scope').selectOption('archived');
  await page.getByRole('checkbox').first().check();
  await page.getByRole('button',{name:'Delete',exact:true}).click();
  const confirmation = page.getByRole('dialog',{name:'Delete selected conversations?'});
  await confirmation.getByRole('button',{name:'Cancel',exact:true}).click();
  expect(await page.evaluate(() => (window as any).storageMutations.length)).toBe(1);
  await page.getByLabel('Conversation scope').selectOption('all');
  await page.getByRole('button',{name:'Select All',exact:true}).click();
  await expect(page.getByText('103 selected',{exact:true})).toBeVisible();
  await page.setViewportSize({width:390,height:844});
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  await page.screenshot({path:info.outputPath('mobile-storage-selection.png')});
  await page.getByRole('button',{name:'Delete',exact:true}).click();
  await confirmation.getByRole('button',{name:'Confirm',exact:true}).click();
  await expect.poll(() => page.evaluate(() => (window as any).storageMutations[1]?.ids.length)).toBe(103);
  await expect(page.getByRole('checkbox')).toHaveCount(0);
});
