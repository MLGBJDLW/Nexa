import { test, expect } from '@playwright/test';

test('phone conversation actions stay compact at narrow widths', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 780 });
  await page.goto('/e2e/fixtures/mobile-shell.html?kind=chat');
  await page.locator('.mobile-app').getByRole('button', { name: 'Conversation', exact: true }).click();
  const row = page.locator('.mobile-conversation-row').first();
  const bounds = await row.boundingBox();
  expect(bounds!.height).toBeLessThanOrEqual(56);
  const title = await row.getByRole('button', { name: 'Previous work', exact: true }).boundingBox();
  expect(title!.width).toBeGreaterThan(130);
});

test('phone web providers expose usable controls without horizontal overflow', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 780 });
  await page.goto('/e2e/fixtures/mobile-shell.html?kind=web');
  const section = page.getByTestId('web-search-settings');
  // Phone settings should expose the feature instead of burying it in a
  // second desktop accordion inside the settings sheet.
  await expect(section).toBeVisible();
  const provider = page.getByTestId('web-search-source-deepseek');
  await provider.locator('button[aria-controls]').first().click();
  const enabled = provider.getByRole('combobox');
  await enabled.selectOption('enabled');
  const key = provider.locator('input[type=password]');
  await key.fill('fixture-search-key');
  expect((await key.boundingBox())!.width).toBeGreaterThan(200);
  const row = await provider.boundingBox();
  expect(row!.x + row!.width).toBeLessThanOrEqual(360);
  await section.getByRole('button', { name: 'Save', exact: true }).click();
  expect(await page.evaluate(() => (window as any).__MOBILE_SETTINGS__.findLast((call: any) => call.args.action === 'save').args.webSearch.customProviders.find((provider: any) => provider.preset === 'deepseek'))).toMatchObject({ enabled: true, apiKey: 'fixture-search-key' });
});

test('phone allows custom dictation and read-aloud without an Android recognition engine', async ({page}, info) => {
  await page.setViewportSize({width:360,height:780}); await page.goto('/e2e/fixtures/mobile-shell.html?kind=speech');
  const section=page.getByTestId('phone-speech-settings');
  await section.locator('select').first().selectOption('cloud');
  const input=page.getByTestId('phone-cloud-input');
  await input.locator('input[type=password]').fill('fixture-input-key');
  await section.getByLabel('Read-aloud service',{exact:true}).selectOption('cloud');
  await page.getByTestId('phone-cloud-output').locator('input[type=password]').fill('fixture-output-key');
  await section.getByRole('button',{name:'Save',exact:true}).click();
  const saved=await page.evaluate(()=>(window as any).__MOBILE_SETTINGS__.findLast((call:any)=>call.args.action==='configure').args);
  expect(saved).toMatchObject({mode:'cloud',outputMode:'cloud',speechToText:{apiKey:'fixture-input-key'},textToSpeech:{apiKey:'fixture-output-key'}});
  await section.getByRole('button',{name:'Test microphone',exact:true}).click();
  await section.getByRole('button',{name:'Stop',exact:true}).click();
  await expect(section.getByText('你好，手机自配语音已转写',{exact:true})).toBeVisible();
  expect(await section.evaluate(node=>node.scrollWidth-node.clientWidth)).toBeLessThanOrEqual(1);
  await page.locator('.mobile-dialog').evaluate(node => { node.scrollTop = 0; });
  await page.screenshot({path:info.outputPath('phone-cloud-speech.png')});
});

test('phone storage manager uses a reachable sheet and normal checkboxes',async({page})=>{
  await page.setViewportSize({width:320,height:700});await page.goto('/e2e/fixtures/mobile-shell.html?kind=storage');
  const manager=page.getByTestId('conversation-storage-manager');
  await manager.getByRole('checkbox').check();
  const checkbox=await manager.getByRole('checkbox').boundingBox(); expect(checkbox!.width).toBeLessThanOrEqual(24);
  await manager.getByRole('button',{name:'Archive',exact:true}).click();
  expect(await page.evaluate(()=>(window as any).__MOBILE_SETTINGS__.filter((call:any)=>call.action==='archive'))).toEqual([{action:'archive',ids:['chat']}]);
});

test('left swipe offers archive without dismissing the history drawer',async({page})=>{
  await page.setViewportSize({width:360,height:780});await page.goto('/e2e/fixtures/mobile-shell.html?kind=chat');
  await page.locator('.mobile-app').getByRole('button',{name:'Conversation',exact:true}).click();
  const row=page.locator('.mobile-conversation-row').first();await row.hover();const box=(await row.boundingBox())!;
  await page.mouse.move(box.x+135,box.y+24);await page.mouse.down();await page.mouse.move(box.x+24,box.y+24,{steps:8});await page.mouse.up();
  await expect(page.getByRole('dialog',{name:'Archive',exact:true})).toBeVisible();
  await page.getByRole('dialog',{name:'Archive',exact:true}).getByRole('button',{name:'Archive',exact:true}).click();
  await expect(page.getByRole('dialog',{name:'Conversation',exact:true})).toBeVisible();
  await expect(page.getByRole('button',{name:'Archive: Previous work',exact:true})).toHaveCount(0);
});
