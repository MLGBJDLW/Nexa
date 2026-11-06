import { test, expect } from '@playwright/test';

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
  const bars = page.locator('.nexa-ui-bar-track');
  expect((await bars.first().boundingBox())!.width).toBeGreaterThan(100);
  const frame=page.getByTestId('html-preview').locator('iframe').contentFrame();
  await expect(frame.getByRole('heading',{name:'HTML 图表'})).toBeVisible();
  expect(await frame.locator('body').evaluate(node=>node.scrollWidth-node.clientWidth)).toBeLessThanOrEqual(1);
  await page.getByTestId('generative-ui').screenshot({path:info.outputPath(`mobile-chart-${width}.png`)});
});
