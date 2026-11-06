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

test('changing the UI language keeps an active speech test usable',async({page})=>{
  await page.setViewportSize({width:360,height:780});await page.goto('/e2e/fixtures/mobile-shell.html?kind=speech');
  const section=page.getByTestId('phone-speech-settings');
  await section.locator('select').first().selectOption('cloud');
  await page.getByTestId('phone-cloud-input').locator('input[type=password]').fill('fixture-input-key');
  await section.getByRole('button',{name:'Test microphone',exact:true}).click();
  await expect(section.getByRole('button',{name:'Stop',exact:true})).toBeVisible();
  const cancelled=await page.evaluate(()=>(window as any).__MOBILE_SETTINGS__.filter((call:any)=>call.args?.action==='cancel').length);
  await page.getByRole('button',{name:'Switch UI language'}).click();
  await expect(section.getByRole('button',{name:'停止',exact:true})).toBeVisible();
  expect(await page.evaluate(()=>(window as any).__MOBILE_SETTINGS__.filter((call:any)=>call.args?.action==='cancel').length)).toBe(cancelled);
  await section.getByRole('button',{name:'停止',exact:true}).click();
  await expect(section.getByRole('button',{name:'保存',exact:true})).toBeEnabled();
  await expect(section.getByText('你好，手机自配语音已转写',{exact:true})).toBeVisible();
});

test('phone TTS model changes replace incompatible preset voices and retain private voices',async({page})=>{
  await page.setViewportSize({width:360,height:780});await page.goto('/e2e/fixtures/mobile-shell.html?kind=speech');
  const section=page.getByTestId('phone-speech-settings');
  await section.getByLabel('Read-aloud service',{exact:true}).selectOption('cloud');
  const output=page.getByTestId('phone-cloud-output');
  await output.locator('select').selectOption('dashscope-cosyvoice');
  const model=output.locator('input[list$="-models"]'),voice=output.locator('input[list$="-voices"]');
  await expect(voice).toHaveValue('longanhuan_v3.6');
  await model.fill('qwen-audio-3.0-tts-plus');
  await expect(voice).not.toHaveValue('longanhuan_v3.6');
  await expect(voice).not.toHaveValue('');
  await voice.fill('private-user-voice');
  await model.fill('qwen-audio-3.0-tts-flash');
  await expect(voice).toHaveValue('private-user-voice');
});

test('left swipe offers archive without dismissing the history drawer',async({page})=>{
  await page.setViewportSize({width:360,height:780});await page.goto('/e2e/fixtures/mobile-shell.html?kind=chat');
  await page.locator('.mobile-app').getByRole('button',{name:'Conversation',exact:true}).click();
  const row=page.locator('.mobile-conversation-row').first();await row.hover();const box=(await row.boundingBox())!;
  await page.mouse.move(box.x+135,box.y+24);await page.mouse.down();await page.mouse.move(box.x+85,box.y+24,{steps:4});
  const reveal=page.locator('.mobile-swipe-action').first();
  const initial=await reveal.evaluate(node=>({width:node.getBoundingClientRect().width,color:getComputedStyle(node).backgroundColor,scale:getComputedStyle(node.querySelector('svg')!).transform}));
  await page.mouse.move(box.x+24,box.y+24,{steps:6});
  const full=await reveal.evaluate(node=>({width:node.getBoundingClientRect().width,color:getComputedStyle(node).backgroundColor,scale:getComputedStyle(node.querySelector('svg')!).transform}));
  expect(full.width).toBeGreaterThan(initial.width);expect(full.color).not.toBe(initial.color);expect(full.scale).not.toBe(initial.scale);
  await page.mouse.up();
  await expect(page.getByRole('dialog',{name:'Archive',exact:true})).toBeVisible();
  await page.getByRole('dialog',{name:'Archive',exact:true}).getByRole('button',{name:'Archive',exact:true}).click();
  await expect(page.getByRole('dialog',{name:'Conversation',exact:true})).toBeVisible();
  await expect(page.getByRole('button',{name:'Archive: Previous work',exact:true})).toHaveCount(0);
});


test('phone settings search reveals matching sections and keeps other groups collapsed',async({page},info)=>{
  await page.setViewportSize({width:360,height:780});await page.goto('/e2e/fixtures/mobile-shell.html?kind=all');
  const search=page.getByRole('searchbox',{name:'Search settings'});
  await expect(search).toBeVisible();
  await expect(page.getByTestId('phone-speech-settings')).not.toBeVisible();
  await search.fill('voice');
  await expect(page.getByTestId('phone-speech-settings')).toBeVisible();
  await expect(page.locator('.mobile-settings-group:visible')).toHaveCount(1);
  await search.fill('unmatched-fixture');await expect(page.getByText('No matching settings.')).toBeVisible();
  await search.fill('');
  await page.locator('.mobile-settings-group > summary').filter({hasText:'Voice'}).click();
  await expect(page.getByTestId('phone-speech-settings')).toBeVisible();
  expect(await page.locator('.mobile-dialog').evaluate(node=>node.scrollWidth-node.clientWidth)).toBeLessThanOrEqual(1);
  await page.screenshot({path:info.outputPath('mobile-settings-search.png')});
});

test('delete all archived chats includes unloaded pages and preserves active chats',async({page})=>{
  await page.goto('/e2e/fixtures/mobile-shell.html?kind=chat');
  await expect(page.locator('.mobile-app')).toBeVisible();
  await page.evaluate(()=>(window as any).__MOBILE_CONTINUITY__.seedArchives());
  await page.locator('.mobile-app').getByRole('button',{name:'Conversation',exact:true}).click();
  await page.getByText('Archived conversations',{exact:true}).click();
  await expect(page.getByRole('button',{name:'Unarchive: Archived a',exact:true})).toBeVisible();
  await expect(page.getByRole('button',{name:'Unarchive: Archived e',exact:true})).toHaveCount(0);
  await page.getByRole('button',{name:'Delete All',exact:true}).click();
  const confirm=page.getByRole('dialog',{name:'Delete All',exact:true});
  await expect(confirm.getByText(/all 5 conversations/)).toBeVisible();
  await page.evaluate(()=>(window as any).__MOBILE_CONTINUITY__.restoreArchived('c'));
  await confirm.getByRole('button',{name:'Delete All',exact:true}).click();
  await expect(confirm).not.toBeVisible();
  await expect(page.getByRole('button',{name:'Previous work',exact:true})).toBeVisible();
  await expect(page.getByRole('button',{name:'Archived c',exact:true})).toBeVisible();
  expect(await page.evaluate(()=>(window as any).__MOBILE_CONTINUITY__.actions.filter((action:any)=>action.method==='chat.deleteArchived').map((action:any)=>action.params.conversationId))).toEqual(['a','b','c','d','e']);
});
