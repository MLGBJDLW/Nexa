import { test, expect } from '@playwright/test';

test('phone steers its active run and accepts a follow-up into an editable draft', async ({page}) => {
  await page.setViewportSize({width:360,height:780});
  await page.goto('/e2e/fixtures/mobile-shell.html?kind=chat');
  const input = page.getByRole('textbox', {name:'Message', exact:true});
  await input.fill('Review this report');
  await page.getByRole('button', {name:'Send', exact:true}).click();
  await expect(input).toHaveAttribute('placeholder', 'Steering message');
  await input.fill('Check the totals first.');
  await page.getByRole('button', {name:'Steering message',exact:true}).click();
  await expect(page.locator('article.remote-steering')).toContainText('Check the totals first.');
  await expect(input).toHaveValue('');
  await expect(page.getByRole('button',{name:'Stop',exact:true})).toBeVisible();
  const next = 'Compare the two report sections.';
  const content = 'All done.\n\n```nexa-followup json\n' + JSON.stringify([{label:'Compare sections',prompt:next}]) + '\n```';
  await page.evaluate(value => {
    (window as any).__MOBILE_CONTINUITY__.emit('outputSnapshot',{channel:'answer',blockId:'answer',text:value.slice(0,-3)});
  }, content);
  await expect(page.locator('[data-timeline-kind="answer"]')).toContainText('All done.');
  await expect(page.locator('pre')).toHaveCount(0);
  await page.evaluate(value => (window as any).__MOBILE_CONTINUITY__.finish(value), content);
  await expect(page.getByTestId('followup-suggestions').getByRole('button',{name:'Compare sections'})).toBeVisible();
  await expect(input).toHaveAttribute('placeholder', next);
  await page.getByRole('button', {name:'Edit draft',exact:true}).click();
  await expect(input).toHaveValue(next);
  expect(await page.evaluate(() => (window as any).__MOBILE_CONTINUITY__.actions.filter((call:any) => call.method === 'chat.start').length)).toBe(1);
});

test('native phone groups activity once per turn during the run and after reloading history', async ({page}, info) => {
  test.slow();
  await page.setViewportSize({width:360,height:780});
  await page.goto('/e2e/fixtures/mobile-shell.html?kind=chat&activity');
  const groups = page.getByTestId('remote-turn-activity');
  const earlierAnswer = page.getByText('第一份报表的合计金额一致。', {exact:true});
  const latestAnswer = page.getByText('第二份报表有三个部分。', {exact:true});
  const approval = page.getByRole('button', {name:'允许本次',exact:true});
  await expect(earlierAnswer).toBeVisible({timeout:45_000});
  await expect(latestAnswer).toBeVisible();
  await expect(approval).toBeVisible();
  const checkCollapsed = async () => {
    await expect(groups).toHaveCount(2);
    await expect(groups.first()).toHaveJSProperty('open', false);
    await expect(groups.last()).toHaveJSProperty('open', false);
    const steering = page.getByText('请重点核对合计金额。', {exact:true});
    await expect(steering).toBeVisible();
    // The clarification arrived after the first tool call.
    expect(await steering.evaluate(node => Boolean(node.compareDocumentPosition(
      document.querySelector('[data-testid="remote-turn-activity"]')!,
    ) & Node.DOCUMENT_POSITION_PRECEDING))).toBe(true);
    await expect(groups.getByText('请重点核对合计金额。', {exact:true})).toHaveCount(0);
    await expect(earlierAnswer).toBeVisible();
    await expect(latestAnswer).toBeVisible();
    await expect(groups.getByText('第一份报表的合计金额一致。', {exact:true})).toHaveCount(0);
    await expect(groups.getByText('第二份报表有三个部分。', {exact:true})).toHaveCount(0);
  };
  const checkExpansion = async (capture?: string) => {
    for (const [index, expected] of [['first', ['thinking','tool','thinking','tool']], ['last', ['thinking','tool']]] as const) {
      const group = index === 'first' ? groups.first() : groups.last();
      await group.locator(':scope > summary').click();
      const rows = group.locator('[data-timeline-kind]');
      await expect.poll(() => rows.evaluateAll(items => items.map(item => item.getAttribute('data-timeline-kind')))).toEqual(expected);
      const labels = index === 'first'
        ? ['读取第一份报表。', 'read_file', '核对表格里的金额。', 'search_table']
        : ['读取第二份报表的目录。', 'read_section'];
      for (const [i, label] of labels.entries()) await expect(rows.nth(i).getByText(label, {exact:true})).toBeVisible();
      if (capture) await group.screenshot({path:info.outputPath(`${capture}-expanded-${index}.png`)});
      await group.locator(':scope > summary').click();
    }
  };
  await checkCollapsed();
  await page.screenshot({path:info.outputPath('live-activity-and-approval.png')});
  await checkExpansion('live');
  await approval.click();
  await expect(approval).toHaveCount(0);
  await expect(page.getByTestId('remote-run-status')).toContainText('已完成');
  await checkCollapsed();
  await checkExpansion();
  await page.reload();
  await checkCollapsed();
  await page.screenshot({path:info.outputPath('stored-turns.png')});
  await checkExpansion('stored');
});

test('paged tool activity joins live replay only for the same prompt', async ({page}) => {
  for (const owner of ['same','different']) {
    await page.goto('/e2e/fixtures/mobile-shell.html?kind=chat&pagedActivity=' + owner);
    const groups = page.getByTestId('remote-turn-activity');
    await expect(groups).toHaveCount(owner === 'same' ? 1 : 2);
    await groups.first().locator(':scope > summary').click();
    await expect(groups.first().getByText('saved_step', {exact:true})).toHaveCount(30);
    if (owner === 'different') {
      await expect(groups.first().getByText('active_step', {exact:true})).toHaveCount(0);
      await groups.last().locator(':scope > summary').click();
    }
    await expect(groups.last().getByText('active_step', {exact:true})).toBeVisible();
  }
});

for (const width of [320, 360, 412, 720]) test(`mobile markdown and charts fit ${width}px`, async ({page}, info) => {
  await page.setViewportSize({width,height:width >= 700 ? 360 : 820});
  await page.goto('/e2e/fixtures/mobile-rendering.html');
  await expect(page.getByTestId('generative-ui')).toBeVisible();
  await expect(page.getByTestId('mermaid-surface').locator('svg')).toBeVisible();
  expect(await page.locator('.native-chat-content').evaluate(node=>node.scrollWidth-node.clientWidth)).toBeLessThanOrEqual(1);
  expect(await page.locator('.nexa-ui').evaluate(node=>node.scrollWidth-node.clientWidth)).toBeLessThanOrEqual(1);
  const table = page.locator('.prose-chat > div > table').nth(1);
  const widths = await table.evaluate(node => ({table:node.getBoundingClientRect().width, header:node.querySelector('thead')!.getBoundingClientRect().width, row:node.querySelector('tbody tr')!.getBoundingClientRect().width, container:node.parentElement!.clientWidth}));
  expect(Math.abs(widths.table-widths.container)).toBeLessThanOrEqual(1);
  expect(Math.abs(widths.header-widths.table)).toBeLessThanOrEqual(2);
  expect(Math.abs(widths.row-widths.table)).toBeLessThanOrEqual(2);
  const bars = page.locator('.nexa-ui-bar-track');
  expect((await bars.first().boundingBox())!.width).toBeGreaterThan(100);
  const frame=page.getByTestId('html-preview').locator('iframe').contentFrame();
  await expect(frame.getByRole('heading',{name:'HTML 图表'})).toBeVisible();
  expect(await frame.locator('body').evaluate(node=>node.scrollWidth-node.clientWidth)).toBeLessThanOrEqual(1);
  await page.getByTestId('generative-ui').screenshot({path:info.outputPath(`mobile-chart-${width}.png`)});
});


test('phone image and camera attachments appear in chat and survive reloading history', async ({page}) => {
  await page.setViewportSize({width:360,height:780});
  await page.goto('/e2e/fixtures/mobile-shell.html?kind=chat&composer&media');
  await page.getByRole('button',{name:'Attach files',exact:true}).click();
  const image = {name:'photo.png',mimeType:'image/png',buffer:Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=','base64')};
  const picker = page.waitForEvent('filechooser');
  await page.getByRole('button',{name:'Attach image',exact:true}).click();
  await (await picker).setFiles(image);
  const camera = page.locator('input[capture="environment"]');
  await expect(camera).toHaveAttribute('accept','image/*');
  const capture = page.waitForEvent('filechooser');
  await page.getByRole('button',{name:'Camera',exact:true}).click();
  await (await capture).setFiles({...image,name:'camera.png'});
  await page.getByRole('textbox',{name:'Message',exact:true}).fill('Photo');
  await page.getByRole('button',{name:'Send',exact:true}).click();
  for (const name of ['photo.png','camera.png']) {
    const thumbnail=page.locator('article.remote-message-user').getByRole('img',{name,exact:true});
    await expect(thumbnail).toBeVisible();
    await expect.poll(()=>thumbnail.evaluate((node:HTMLImageElement)=>node.naturalWidth)).toBeGreaterThan(0);
  }
  await page.reload();
  const restored=page.locator('article.remote-message-user').getByRole('img',{name:'photo.png',exact:true});
  await expect(restored).toBeVisible();
  await restored.click();
  await expect(page.getByTestId('image-lightbox')).toBeVisible();
});
