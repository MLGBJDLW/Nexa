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

test('phone cloud dictation and read-aloud reuse synced Bailian accounts', async ({page}) => {
  await page.goto('/e2e/fixtures/mobile-shell.html?kind=speech&synced');
  const section = page.getByTestId('phone-speech-settings');
  await section.locator('select').first().selectOption('cloud');
  await section.getByLabel('Read-aloud service', {exact:true}).selectOption('cloud');
  const input = section.getByTestId('phone-cloud-input');
  const output = section.getByTestId('phone-cloud-output');
  for (const fields of [input, output]) {
    const account = fields.getByRole('combobox', {name:'Model connection', exact:true});
    await expect(account).toHaveValue('synced-0');
    await expect(account.locator('option:checked')).toHaveText('Personal Bailian');
    await expect(fields.locator('input[type=password]')).toHaveCount(0);
    await expect(fields.getByLabel('Model', {exact:true})).not.toHaveValue('');
  }
  await output.getByLabel('Model connection', {exact:true}).selectOption('synced-1');
  await section.getByRole('button', {name:'Save', exact:true}).click();
  const saved = await page.evaluate(() => (window as any).__MOBILE_SETTINGS__.findLast((call:any) => call.args.action === 'configure').args);
  expect(saved).toMatchObject({speechToText:{connectionId:'synced-0',apiKey:'',apiStyle:'dashscope_asr'},textToSpeech:{connectionId:'synced-1',apiKey:'',apiStyle:'dashscope_speech'}});
  await expect(output.getByLabel('Model connection', {exact:true})).toHaveValue('synced-1');
  await input.getByLabel('Model connection', {exact:true}).selectOption('');
  await expect(input.locator('input[type=password]')).toBeVisible();
  await expect(output.locator('input[type=password]')).toHaveCount(0);
});

test('existing speech settings retain manual keys and recognize inherited accounts', async ({page}) => {
  await page.goto('/e2e/fixtures/mobile-shell.html?kind=speech&synced&legacy');
  const input = page.getByTestId('phone-cloud-input');
  const output = page.getByTestId('phone-cloud-output');
  await expect(input.getByLabel('Model connection', {exact:true})).toHaveValue('');
  await expect(input.locator('input[type=password]')).toHaveValue('__NEXA_SAVED_KEY__');
  await expect(output.getByLabel('Model connection', {exact:true})).toHaveValue('synced-0');
  await expect(output.locator('input[type=password]')).toHaveCount(0);
  await expect(output.getByLabel('Model', {exact:true})).toHaveValue('private-tts');
  await expect(output.locator('input[list$="-voices"]')).toHaveValue('private-voice');
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
  const reveal=row.getByRole('button', {name: 'Archive: Previous work'});
  await expect(row.locator('svg.lucide-archive')).toHaveCount(1);
  const initial=await reveal.evaluate(node=>({width:node.getBoundingClientRect().width,color:getComputedStyle(node).backgroundColor,scale:getComputedStyle(node.querySelector('svg')!).transform}));
  await page.mouse.move(box.x+24,box.y+24,{steps:6});
  const full=await reveal.evaluate(node=>({width:node.getBoundingClientRect().width,color:getComputedStyle(node).backgroundColor,scale:getComputedStyle(node.querySelector('svg')!).transform}));
  expect(full.width).toBeGreaterThan(initial.width);expect(full.color).not.toBe(initial.color);expect(full.scale).not.toBe(initial.scale);
  await page.mouse.up();
  await expect(page.getByRole('dialog',{name:'Archive',exact:true})).toBeVisible();
  await expect(reveal).toHaveCSS('width', '104px');
  // Capture rendered frames so an instant unmount cannot pass as a transition.
  const collapse = row.evaluate(element => new Promise<boolean>(resolve => {
    const wrapper = element.closest('.mobile-swipe-row')!;
    const start = wrapper.getBoundingClientRect().height;
    let shrank = false;
    const frame = () => {
      if (!wrapper.isConnected) { resolve(shrank); return; }
      const height = wrapper.getBoundingClientRect().height;
      if (height > 0 && height < start - 1) shrank = true;
      requestAnimationFrame(frame);
    };
    frame();
  }));
  await page.getByRole('dialog',{name:'Archive',exact:true}).getByRole('button',{name:'Archive',exact:true}).click();
  await expect(page.getByRole('dialog',{name:'Conversation',exact:true})).toBeVisible();
  await expect(page.getByRole('button',{name:'Archive: Previous work',exact:true})).toHaveCount(0);
  expect(await collapse).toBe(true);
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

test('archived rows restore to the right and confirm deletion to the left',async({page})=>{
  await page.setViewportSize({width:360,height:780});
  await page.goto('/e2e/fixtures/mobile-shell.html?kind=chat');
  await expect(page.locator('.mobile-app')).toBeVisible();
  await page.evaluate(()=>(window as any).__MOBILE_CONTINUITY__.seedArchives());
  await page.getByRole('button',{name:'Conversation',exact:true}).click();
  await page.getByRole('button',{name:'Archived conversations',exact:true}).click();
  const swipe = async (id: string, dx: number) => {
    const row = page.locator(`[data-conversation-id="${id}"]`).last();
    const box = (await row.boundingBox())!;
    const x = box.x + box.width / 2, y = box.y + box.height / 2;
    await page.mouse.move(x,y); await page.mouse.down();
    await page.mouse.move(x+dx,y,{steps:8}); await page.mouse.up();
  };
  await swipe('a',95);
  await expect(page.getByRole('button',{name:'Archived a',exact:true})).toBeVisible();
  await swipe('b',-95);
  const confirm=page.getByRole('dialog',{name:'Delete',exact:true});
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button',{name:'Delete',exact:true}).click();
  await expect(page.getByRole('button',{name:'Unarchive: Archived b'})).toHaveCount(0);
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
  await page.evaluate(()=>{const app=(window as any).__MOBILE_CONTINUITY__;app.restoreArchived('c');app.removeArchived('d');});
  await confirm.getByRole('button',{name:'Delete All',exact:true}).click();
  await expect(confirm).not.toBeVisible();
  await expect(page.getByRole('button',{name:'Previous work',exact:true})).toBeVisible();
  await expect(page.getByRole('button',{name:'Archived c',exact:true})).toBeVisible();
  expect(await page.evaluate(()=>(window as any).__MOBILE_CONTINUITY__.actions.filter((action:any)=>action.method==='chat.deleteArchived').map((action:any)=>action.params.conversationId))).toEqual(['a','b','c','d','e']);
});


test('holding the phone composer dictates into the draft and Plan is a distinct control',async({page})=>{
  await page.setViewportSize({width:320,height:740});
  await page.goto('/e2e/fixtures/mobile-shell.html?kind=chat&composer=1');
  const input=page.getByRole('textbox',{name:'Message',exact:true});
  const plan=page.getByRole('button',{name:'Plan',exact:true});
  const mic=page.getByRole('button',{name:'Voice input',exact:true});
  await expect(plan).toHaveAttribute('aria-pressed','false');
  await plan.focus(); await page.keyboard.press('Space'); await expect(plan).toHaveAttribute('aria-pressed','true');
  await page.emulateMedia({reducedMotion:'reduce'});
  expect(await plan.locator('svg').evaluate(node=>getComputedStyle(node).transform)).toBe('none');
  for (const width of [320,360]) {
    await page.setViewportSize({width,height:740});
    for (const control of [plan,mic,page.getByRole('button',{name:'Send',exact:true})]) {
      const bounds=(await control.boundingBox())!; expect(bounds.x).toBeGreaterThanOrEqual(0); expect(bounds.x+bounds.width).toBeLessThanOrEqual(width);
    }
  }
  const plus=page.locator('.native-composer-actions').getByRole('button',{name:'Attach files',exact:true});await plus.click();
  await expect(page.getByRole('combobox',{name:'Execution mode'})).toHaveCount(0); await plus.first().click();
  const driver=await page.context().newCDPSession(page);
  const touchStart=async()=>{
    const box=(await input.boundingBox())!;
    // Press blank editing space; pressing directly on a word retains the native selection menu.
    await driver.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x:box.x+box.width-15,y:box.y+14}]});
  };
  const release=(type:'touchEnd'|'touchCancel')=>driver.send('Input.dispatchTouchEvent',{type,touchPoints:[]});
  const count=(method:string)=>page.evaluate(method=>(window as any).__MOBILE_CONTINUITY__.actions.filter((a:any)=>a.method===method).length,method);
  await touchStart();await release('touchEnd');expect(await count('speech.listen')).toBe(0);
  // Allow the short tap's double-click selection window to expire.
  await page.waitForTimeout(380);
  await touchStart();await expect.poll(()=>count('speech.listen')).toBe(1);await release('touchEnd');
  await expect(input).toHaveValue('Hello from dictation'); expect(await count('speech.finish')).toBe(1);expect(await count('chat.start')).toBe(0);
  await input.fill('draft');await input.evaluate(node=>node.setSelectionRange(5,5));
  await touchStart();await expect.poll(()=>count('speech.listen')).toBe(2);await release('touchCancel');
  await expect.poll(()=>count('speech.cancel')).toBe(1);await expect(input).toHaveValue('draft');
  await mic.click();await expect.poll(()=>count('speech.listen')).toBe(3);await page.getByRole('button',{name:'Stop',exact:true}).click();
  await expect(input).toHaveValue('draft Hello from dictation');
  await page.getByRole('button',{name:'Send',exact:true}).click();
  const started=await page.evaluate(()=>(window as any).__MOBILE_CONTINUITY__.actions.find((a:any)=>a.method==='chat.start'));
  expect(started.params.executionMode).toBe('plan');
});


test('phone schedules are created in the selected chat and changes carry its revision', async ({page}) => {
  await page.setViewportSize({width:360,height:780});
  await page.goto('/e2e/fixtures/mobile-shell.html?kind=chat');
  await page.getByRole('button',{name:'Conversation',exact:true}).click();
  await page.getByRole('button',{name:'Scheduled tasks',exact:true}).click();
  const sheet=page.getByRole('dialog',{name:'Scheduled tasks',exact:true});
  await sheet.getByRole('button',{name:'Create automation'}).click();
  await sheet.getByLabel('Name',{exact:true}).fill('Morning check');
  await sheet.getByLabel('Prompt',{exact:true}).fill('Summarize the overnight changes.');
  await sheet.getByLabel('Timezone (IANA)',{exact:true}).fill('Asia/Shanghai');
  await sheet.getByRole('button',{name:'Save automation'}).click();
  await expect(sheet.getByText('Morning check',{exact:true})).toBeVisible();
  await sheet.getByRole('button',{name:/pause/i}).click();
  await expect(sheet.getByText('Paused',{exact:true})).toBeVisible();
  const calls=await page.evaluate(()=>(window as any).__MOBILE_CONTINUITY__.actions.filter((x:any)=>x.method==='chat.schedules'));
  const created=calls.find((x:any)=>x.params.input.action==='create').params;
  expect(created.conversationId).toBeTruthy();
  expect(calls.find((x:any)=>x.params.input.action==='pause').params).toEqual({conversationId:created.conversationId,input:{action:'pause',id:'job',expectedRevision:1}});
});
