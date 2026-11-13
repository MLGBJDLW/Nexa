import { expect, test, type Page } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('nexa-locale', 'en'));
});
const state = (page: Page) => page.evaluate(() => {
  const value = (window as any).__screenFixture;
  return { frames: value.frames, pending: value.pending, maxPending: value.maxPending, inputs: value.inputs, stops: value.stops, acceptedInputs:value.acceptedInputs };
});
async function open(page: Page, phone = false) {
  await page.goto(`/e2e/fixtures/device-screen.html${phone ? '?phone' : ''}`);
  await expect(page.getByRole('img', { name: 'Device screen' })).toBeVisible();
}

test('scaled desktop input preserves the clicked observation and waits for one pending frame', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await open(page);
  await page.getByRole('checkbox').check();
  await page.evaluate(() => { (window as any).__screenFixture.hold = true; });
  await expect.poll(async () => (await state(page)).pending).toBe(1);
  const screenshot = page.getByRole('img', { name: 'Device screen' });
  const box = (await screenshot.boundingBox())!;
  await screenshot.click({ position: { x: box.width * .25, y: box.height * .75 } });
  expect((await state(page)).inputs).toHaveLength(0);
  await page.evaluate(() => (window as any).__releaseScreen());
  await expect.poll(async () => (await state(page)).inputs.length).toBe(1);
  const input = (await state(page)).inputs[0];
  expect(input.action).toBe('click'); expect(input.observation_id).toBe('first:1'); expect(input.window_id).toBe(42);
  expect(input.x).toBeCloseTo(320, -1); expect(input.y).toBeCloseTo(540, -1);
  expect((await state(page)).maxPending).toBe(1);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  await page.getByRole('button', { name: 'Scroll down' }).click();
  await expect.poll(async () => (await state(page)).inputs.length).toBe(2);
  expect((await state(page)).inputs[1]).toMatchObject({action:'scroll',scroll_y:3,window_id:42});
  expect((await state(page)).inputs[1].x).toBeCloseTo(320,-1);
  expect((await state(page)).inputs[1].y).toBeCloseTo(540,-1);
});

test('desktop scrolling targets the shared image center before a point is selected', async ({ page }) => {
  await open(page);
  await page.getByRole('checkbox').check();
  await page.getByRole('button', { name: 'Scroll up' }).click();
  await expect.poll(async () => (await state(page)).inputs.length).toBe(1);
  expect((await state(page)).inputs[0]).toMatchObject({action:'scroll',scroll_y:-3,x:640,y:360});
});

test('stop suppresses late frames and never replays queued input', async ({ page }) => {
  await open(page);
  await page.getByRole('checkbox').check();
  await page.evaluate(() => { (window as any).__screenFixture.hold = true; });
  await expect.poll(async () => (await state(page)).pending).toBe(1);
  await page.getByRole('img', { name: 'Device screen' }).click({ position: { x: 20, y: 20 } });
  await page.getByRole('button', { name: 'Stop sharing' }).click();
  await page.evaluate(() => (window as any).__releaseScreen());
  await expect.poll(async () => (await state(page)).pending).toBe(0);
  await expect(page.getByTestId('device-screen')).toHaveCount(0);
  expect((await state(page)).inputs).toHaveLength(0);
  expect((await state(page)).stops).toBe(1);
  const frames = (await state(page)).frames;
  await page.waitForTimeout(1100);
  expect((await state(page)).frames).toBe(frames);
});

test('paused or failed captures cannot send input and resume requires a fresh frame', async ({ page }) => {
  await open(page);
  await page.getByRole('checkbox').check();
  await page.getByRole('button', { name: 'Pause viewing' }).click();
  const frames = (await state(page)).frames;
  await page.getByRole('img', { name: 'Device screen' }).click({ position: { x: 20, y: 20 } });
  await page.waitForTimeout(1100);
  expect((await state(page)).frames).toBe(frames);
  expect((await state(page)).inputs).toHaveLength(0);
  await page.evaluate(() => { (window as any).__screenFixture.hold = true; (window as any).__screenFixture.failFrame = true; });
  await page.getByRole('button', { name: 'Resume viewing' }).click();
  await expect.poll(async () => (await state(page)).pending).toBe(1);
  await expect(page.getByRole('checkbox')).toBeDisabled();
  await page.getByRole('img', { name: 'Device screen' }).click({ position: { x: 20, y: 20 } });
  expect((await state(page)).inputs).toHaveLength(0);
  await page.evaluate(() => (window as any).__releaseScreen());
  await expect(page.getByRole('alert')).toContainText('Screen disconnected');
  await expect(page.getByRole('checkbox')).toBeDisabled();
  await page.getByRole('img', { name: 'Device screen' }).click({ position: { x: 20, y: 20 } });
  expect((await state(page)).inputs).toHaveLength(0);
  await page.evaluate(() => { (window as any).__screenFixture.failFrame = false; });
  await expect(page.getByRole('alert')).toHaveCount(0);
  await page.getByRole('img', { name: 'Device screen' }).click({ position: { x: 20, y: 20 } });
  await expect.poll(async () => (await state(page)).inputs.length).toBe(1);
});

test('phone text and navigation use native observation identity and show action failures without replay', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await open(page, true);
  await page.getByRole('checkbox').check();
  const text = page.getByRole('textbox');
  await text.fill('你好 — hello');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(async () => (await state(page)).inputs.length).toBe(1);
  expect((await state(page)).inputs[0]).toMatchObject({ action: 'set_text', text: '你好 — hello', nodeId: 7, packageName: 'example.first' });
  await expect(text).toHaveValue('');
  await page.evaluate(() => { (window as any).__screenFixture.failInput = true; });
  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('Screen changed');
  await page.waitForTimeout(1100);
  await expect(page.getByRole('alert')).toContainText('Screen changed');
  expect((await state(page)).inputs).toHaveLength(2);
});

for (const changed of [false, true]) {
  test(`queued phone input ${changed ? 'rejects replaced' : 'preserves unchanged'} native screen evidence`, async ({ page }) => {
    await open(page, true);
    await page.getByRole('checkbox').check();
    await page.evaluate(() => { (window as any).__screenFixture.hold = true; });
    await expect.poll(async () => (await state(page)).pending).toBe(1);
    await page.getByRole('img', { name: 'Device screen' }).click({position:{x:20,y:20}});
    await page.evaluate(changed => { if(changed)(window as any).__screenFixture.phoneRevision++;(window as any).__releaseScreen(); },changed);
    await expect.poll(async () => (await state(page)).inputs.length).toBe(1);
    expect((await state(page)).inputs[0].observationId).toBe('first:screen:0');
    if(changed)await expect(page.getByRole('alert')).toContainText('Screen changed');
    else await expect(page.getByRole('alert')).toHaveCount(0);
    expect((await state(page)).acceptedInputs).toBe(changed?0:1);
    await page.waitForTimeout(1100);
    expect((await state(page)).inputs).toHaveLength(1);
  });
}

test('switching device clears consent and stale frames before another device can receive input', async ({ page }) => {
  await open(page);
  await page.getByRole('checkbox').check();
  await page.evaluate(() => { (window as any).__screenFixture.hold = true; });
  await page.getByRole('button', { name: 'Switch device' }).click();
  await expect(page.getByRole('checkbox')).not.toBeChecked();
  await expect(page.getByRole('img', { name: 'Device screen' })).toHaveCount(0);
  await page.evaluate(() => (window as any).__releaseScreen());
  await expect(page.getByRole('heading', { name: 'Screen second' })).toBeVisible();
  expect((await state(page)).inputs).toHaveLength(0);
});

test('collapsing the desktop phone viewer cancels its native session and queued input', async ({ page }) => {
  await page.goto('/e2e/fixtures/device-screen.html?phone&desktop');
  const toggle = page.getByRole('button', { name: 'Device screen', exact: true });
  await toggle.click();
  await page.getByRole('button', { name: 'View phone screen', exact: true }).click();
  await expect(page.getByRole('img', { name: 'Device screen' })).toBeVisible();
  await page.getByTestId('device-screen').getByRole('checkbox').check();
  await page.evaluate(() => { (window as any).__screenFixture.hold = true; });
  await expect.poll(async () => (await state(page)).pending).toBe(1);
  await page.getByRole('img', { name: 'Device screen' }).click({ position: { x: 20, y: 20 } });
  await toggle.click();
  await expect.poll(async () => (await state(page)).stops).toBe(1);
  await page.evaluate(() => (window as any).__releaseScreen());
  await expect.poll(async () => (await state(page)).pending).toBe(0);
  expect((await state(page)).inputs).toHaveLength(0);
  await toggle.click();
  await expect(page.getByTestId('device-screen')).toHaveCount(0);
});
