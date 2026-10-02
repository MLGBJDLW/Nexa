import { expect,test } from '@playwright/test';

test('progress for one of 512 tool entities leaves unrelated React subscriptions and local state intact',async({page})=>{
  await page.goto('/e2e/fixtures/tool-entities.html');
  const untouched=page.getByTestId('probe-0');
  await untouched.getByRole('button',{name:'Toggle retained panel 0',exact:true}).click();
  await page.getByRole('textbox',{name:'Independent draft'}).fill('Unsent draft');
  const before=await untouched.getAttribute('data-renders');
  await page.evaluate(()=>(window as any).__ENTITY_TEST__.update());
  await expect(page.getByTestId('probe-511').locator('output')).toHaveText('Update 999');
  await expect(untouched).toHaveAttribute('data-renders',before!);
  await expect(page.getByTestId('retained-0')).toBeVisible();
  await expect(page.getByRole('textbox',{name:'Independent draft'})).toHaveValue('Unsent draft');
});
