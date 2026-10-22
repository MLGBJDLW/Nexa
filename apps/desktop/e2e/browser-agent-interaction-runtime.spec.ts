import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { expect, test } from '@playwright/test';

const source = readFileSync(join(process.cwd(), 'src-tauri', 'src', 'browser', 'scripts.rs'), 'utf8');
const runtimeSource = readFileSync(join(process.cwd(), 'src-tauri', 'src', 'browser', 'interaction_runtime.js'), 'utf8')
  .replace('__NEXA_PICK_TOKEN__', JSON.stringify('playwright-picker-token'));
const takeoverSource = source.match(/pub fn browser_takeover_script[\s\S]*?\br#"\r?\n([\s\S]*?)\r?\n"#\r?\n\s*\.replace/)?.[1]
  .replace('__NEXA_TAKEOVER_URL__', JSON.stringify('nexa-user-input://playwright-takeover-token'))
  .replace('__NEXA_TAKEOVER_TOKEN__', JSON.stringify('playwright-takeover-token'));

if (!runtimeSource) throw new Error('Could not extract the native Browser Workspace interaction runtime');
if (!takeoverSource) throw new Error('Could not extract the native Browser Workspace takeover guard');

// Match visible native WebViews: headless Chromium otherwise hides scrollbar chrome.
test.use({ launchOptions: { ignoreDefaultArgs: ['--hide-scrollbars'] } });

test('Browser pages retain draggable vertical and horizontal scrollbars', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 700, height: 500 });
  await page.setContent(`<!doctype html><style>
    html { scrollbar-width: none; }
    *::-webkit-scrollbar { display: none; width: 0; height: 0; }
    body { margin: 0; }
    main { width: 1600px; height: 1800px; background: linear-gradient(135deg, #e0f2fe, #c4b5fd); }
  </style><main>Wide and tall browser page</main>`);
  const gutters = () => page.evaluate(() => ({
    vertical: innerWidth - document.documentElement.clientWidth,
    horizontal: innerHeight - document.documentElement.clientHeight,
  }));
  await expect.poll(gutters).toEqual({ vertical: 0, horizontal: 0 });
  await page.addScriptTag({ content: runtimeSource });
  await expect.poll(gutters).toEqual({ vertical: 12, horizontal: 12 });
  const userEpoch = () => page.evaluate(() => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.observe().userEpoch);
  const beforeDrag = await userEpoch();
  await page.mouse.move(694, 50);
  await page.mouse.down();
  await page.mouse.move(694, 330, { steps: 8 });
  await page.mouse.up();
  await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(400);
  await page.mouse.move(70, 494);
  await page.mouse.down();
  await page.mouse.move(430, 494, { steps: 8 });
  await page.mouse.up();
  await expect.poll(() => page.evaluate(() => scrollX)).toBeGreaterThan(400);
  await expect.poll(userEpoch).toBeGreaterThan(beforeDrag);
  await page.screenshot({ path: testInfo.outputPath('browser-scrollbars.png') });
  // Modal scroll locks belong to the page; restoring chrome must not unlock it.
  await page.evaluate(() => { document.documentElement.style.overflow = 'hidden'; });
  await expect.poll(gutters).toEqual({ vertical: 0, horizontal: 0 });
});

test('Capture without feedback does not depend on animation frames', async ({ page }) => {
  await page.setContent('<button>Target</button>');
  await page.addScriptTag({ content: runtimeSource });
  const status = await page.evaluate(() => {
    window.requestAnimationFrame = () => 1;
    const bridge = (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__;
    bridge.beginCapture('no-feedback');
    return bridge.captureStatus('no-feedback').status;
  });
  expect(status).toBe('ready');
});

test('Already satisfied form actions report no input and dispatch no events', async ({ page }) => {
  await page.setContent('<input type="checkbox" checked aria-label="Ready"><select aria-label="Choice"><option selected value="a">A</option><option value="b">B</option></select>');
  await page.addScriptTag({ content: runtimeSource });
  const results = await page.evaluate(() => {
    const bridge = (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__;
    let events = 0;
    for (const type of ['input', 'change', 'click']) document.addEventListener(type, () => events++);
    const act = (action: string, name: string, extra: object) => {
      const observation = bridge.observe();
      const target = observation.elements.find(element => element.name === name)!;
      return bridge.act({ ...observation, action, targetRef: target.ref, expected: target, button: 'left', modifiers: [], ...extra });
    };
    const checked = act('set_checked', 'Ready', { checked: true });
    const selected = act('select', 'Choice', { value: 'a' });
    const changed = act('select', 'Choice', { value: 'b' });
    return { checked, selected, changed, events };
  });
  expect(results.checked).toEqual({ inputSent: false });
  expect(results.selected).toEqual({ inputSent: false });
  expect(results.changed).toEqual({ inputSent: true });
  expect(results.events).toBe(2);
});

test('Presentation arrival follows the actual animation and ignores old completion', async ({ page }) => {
  await page.setContent('<button style="margin:100px">Target</button>');
  await page.addScriptTag({ content: runtimeSource });
  const observation = await observe(page);
  const input = { ...actionInput(observation, 'click', observation.elements[0].ref), presentationId: 'first' };
  const preview = await page.evaluate(value => {
    const bridge = (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__;
    const result = bridge.previewAction(value);
    document.querySelector('[data-nexa-agent-cursor]')!.getAnimations().forEach(animation => animation.playbackRate = 0.05);
    return result;
  }, input);
  await page.waitForTimeout(preview.durationMs + 80);
  expect(await page.evaluate(() => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.presentationStatus('first').status)).toBe('pending');
  await page.evaluate(() => document.querySelector('[data-nexa-agent-cursor]')!.getAnimations().forEach(animation => animation.finish()));
  await expect.poll(() => page.evaluate(() => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.presentationStatus('first').status)).toBe('arrived');
  await page.evaluate(value => {
    const bridge = (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__;
    bridge.previewAction({ ...value, presentationId: 'second' });
    bridge.cancelPresentation('first');
  }, input);
  expect(await page.evaluate(() => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.presentationStatus('first').status)).toBe('unavailable');
  await page.evaluate(() => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.invalidateForUserTakeover());
  expect(await page.evaluate(() => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.presentationStatus('second').status)).toBe('cancelled');
});

test('Capture excludes agent decoration and restores it after nested capture scopes', async ({ page }, testInfo) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setContent('<body style="margin:0;background:white"><button style="margin:100px">Target</button></body>');
  await page.addScriptTag({ content: runtimeSource });
  const baseline = await page.screenshot();
  const observation = await observe(page);
  const input = { ...actionInput(observation, 'click', observation.elements[0].ref), presentationId: 'capture' };
  await page.evaluate(value => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.previewAction(value), input);
  expect((await page.screenshot()).equals(baseline)).toBe(false);
  await page.evaluate(() => {
    const bridge = (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__;
    bridge.beginCapture('outer'); bridge.beginCapture('inner');
  });
  await expect.poll(() => page.evaluate(() => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.captureStatus('inner').status)).toBe('ready');
  const hidden = await page.screenshot({ path: testInfo.outputPath('feedback-free-capture.png') });
  expect(hidden.equals(baseline)).toBe(true);
  await page.evaluate(() => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.endCapture('inner'));
  await expect(page.locator('[data-nexa-agent-cursor]')).toBeHidden();
  await page.evaluate(() => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.endCapture('outer'));
  await expect(page.locator('[data-nexa-agent-cursor]')).toBeVisible();
  expect((await page.screenshot()).equals(baseline)).toBe(false);
});

test('Agent browser interaction shows cursor motion and commits verified pointer actions', async ({ page }, testInfo) => {
  await page.setContent(`
    <!doctype html>
    <button id="source">Open details</button>
    <button id="destination">Destination</button>
    <script>
      window.actionEvents = [];
      for (const type of ['pointerover', 'pointermove', 'pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click', 'dblclick', 'dragstart', 'dragenter', 'dragover', 'drop', 'dragend']) {
        document.addEventListener(type, event => window.actionEvents.push({ type, target: event.target.id, buttons: event.buttons }), true);
      }
    </script>
  `);
  await page.addScriptTag({ content: runtimeSource });

  const observation = await observe(page);
  const sourceRef = observation.elements.find(element => element.name === 'Open details')?.ref;
  expect(sourceRef).toBeTruthy();
  const input = actionInput(observation, 'click', sourceRef!);
  const preview = await page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.previewAction(value), input);

  expect(preview.durationMs).toBeGreaterThanOrEqual(180);
  await expect(page.locator('[data-nexa-agent-cursor]')).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('browser-link.png') });
  await page.waitForTimeout(preview.durationMs + 20);
  await expect(page.locator('[data-nexa-agent-cursor]')).toHaveCSS('pointer-events', 'none');
  await page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.act(value), input);

  const clickEvents = await page.evaluate(() => (
    window as unknown as { actionEvents: Array<{ type: string; target: string }> }
  ).actionEvents.filter(event => event.target === 'source').map(event => event.type));
  expect(clickEvents).toEqual(expect.arrayContaining([
    'pointerover', 'pointermove', 'pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click',
  ]));
  const buttonStates = await page.evaluate(() => (
    window as unknown as { actionEvents: Array<{ type: string; buttons: number }> }
  ).actionEvents.filter(event => ['pointermove', 'pointerdown', 'pointerup'].includes(event.type)));
  expect(buttonStates.find(event => event.type === 'pointermove')?.buttons).toBe(0);
  expect(buttonStates.find(event => event.type === 'pointerdown')?.buttons).toBe(1);
  expect(buttonStates.find(event => event.type === 'pointerup')?.buttons).toBe(0);
  await expect(page.locator('[data-nexa-agent-click]')).toBeVisible();

  const next = await observe(page);
  const nextSourceRef = next.elements.find(element => element.name === 'Open details')!.ref;
  await page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.act(value), actionInput(next, 'double_click', nextSourceRef));
  expect(await page.evaluate(() => (
    window as unknown as { actionEvents: Array<{ type: string }> }
  ).actionEvents.filter(event => event.type === 'dblclick').length)).toBe(1);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const reducedSnapshot = await observe(page);
  const reducedPreview = await page.evaluate(value => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.previewAction(value), actionInput(reducedSnapshot, 'hover', sourceRef!));
  expect(reducedPreview.durationMs).toBe(0);
  await expect(page.locator('[data-nexa-agent-thread]')).toHaveCount(0);
});

test('screenshot confirmation preserves the element references advertised to the agent', async ({ page }) => {
  await page.setContent('<!doctype html><button onclick="this.textContent=\'Verified\'">Open details</button>');
  await page.addScriptTag({ content: runtimeSource });
  const advertised = await observe(page);
  const ref = advertised.elements.find(element => element.name === 'Open details')!.ref;
  await page.screenshot();
  // This is the native observation path: DOM snapshot, screenshot, then a
  // second snapshot that verifies the captured page before returning refs.
  await observe(page);
  await page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.act(value), actionInput(advertised, 'click', ref));
  expect((await observe(page)).elements.some(element => element.name === 'Verified')).toBe(true);
});

test('an unrelated live counter does not invalidate an unchanged action target', async ({ page }) => {
  await page.setContent('<!doctype html><p id="fps">FPS 60</p><button onclick="this.textContent=\'Paused\'">Pause</button>');
  await page.addScriptTag({ content: runtimeSource });
  const advertised = await observe(page);
  const ref = advertised.elements.find(element => element.name === 'Pause')!.ref;
  await page.locator('#fps').evaluate(el => { el.textContent = 'FPS 59'; });
  await page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.act(value), actionInput(advertised, 'click', ref));
  expect((await observe(page)).elements.some(element => element.name === 'Paused')).toBe(true);
});

for (const owner of ['row', 'card', 'inline']) {
test(`a reused action control cannot act on changed ${owner} context`, async ({ page }) => {
  const contents = '<span id="account" style="display:inline-block;width:120px">Account A</span><span><button aria-label="Delete" onclick="window.deleted=document.getElementById(\'account\').textContent">×</button></span>';
  await page.setContent(`<!doctype html><p id="fps">FPS 60</p>${owner === 'inline' ? contents : `<div ${owner === 'row' ? 'role="row"' : 'class="card"'}>${contents}</div>`}`);
  await page.addScriptTag({ content: runtimeSource });
  const advertised = await observe(page);
  const ref = advertised.elements.find(element => element.name === 'Delete')!.ref;
  await page.locator('#account').evaluate(el => { el.textContent = 'Account B'; });
  // The element, accessible button name and geometry are all unchanged.
  const refreshed = await observe(page);
  expect(refreshed.elements.find(element => element.ref === ref)?.bounds)
    .toEqual(advertised.elements.find(element => element.ref === ref)?.bounds);
  await expect(page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.act(value), actionInput(advertised, 'click', ref))).rejects.toThrow(/stale observation/);
  expect(await page.evaluate(() => (window as Window & { deleted?: string }).deleted)).toBeUndefined();
  await page.locator('#fps').evaluate(el => { el.textContent = 'FPS 58'; });
  await page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.act(value), actionInput(refreshed, 'click', ref));
  expect(await page.evaluate(() => (window as Window & { deleted?: string }).deleted)).toBe('Account B');
});
}

test('trusted clicks reject a row recycled between preparation, arming and commit', async ({ page }) => {
  await page.setContent('<!doctype html><p id="fps">FPS 60</p><div role="row"><a href="#account-a" id="account" style="display:inline-block;width:120px">Account A</a><button id="delete" aria-label="Delete" onclick="window.deleted=true">×</button></div>');
  await page.addScriptTag({ content: runtimeSource });
  await page.addScriptTag({ content: takeoverSource });
  const binding = await targetBinding(page, '#delete');
  const bounds = (await page.locator('#delete').boundingBox())!;
  const expected = { kind: 'pointer' as const, x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2, button: 'left' as const, ...binding };
  const arm = (operationId: string) => page.evaluate(({ expected, operationId }) => (
    window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }
  ).__NEXA_TRUSTED_INPUT_GUARD__.arm('playwright-takeover-token', operationId, { pointerDown: 1, keyDown: 0, input: 0 }, expected), { expected, operationId });
  await page.locator('#account').evaluate(el => { el.textContent = 'Account B'; });
  expect(await arm('stale-preparation')).toBe(false);
  await page.locator('#account').evaluate(el => { el.textContent = 'Account A'; });
  expect(await arm('changed-after-arm')).toBe(true);
  await page.locator('#fps').evaluate(el => { el.textContent = 'FPS 59'; });
  await page.mouse.move(expected.x, expected.y);
  await page.mouse.down();
  await page.locator('#account').evaluate(el => { el.textContent = 'Account B'; });
  await page.mouse.up();
  expect(await page.evaluate(() => (window as Window & { deleted?: boolean }).deleted)).toBeUndefined();
  expect(await page.evaluate(() => (window as unknown as {
    __NEXA_TRUSTED_INPUT_GUARD__: { disarm(token: string, id: string): boolean }
  }).__NEXA_TRUSTED_INPUT_GUARD__.disarm('playwright-takeover-token', 'changed-after-arm'))).toBe(false);
});

for (const key of ['Space', 'Enter']) {
test(`trusted ${key} activation cannot commit against recycled row context`, async ({ page }) => {
  await page.setContent('<!doctype html><div role="row"><a href="#account-a" id="account">Account A</a><button id="delete" onclick="window.deleted=true" onkeydown="document.getElementById(\'account\').textContent=\'Account B\'">Delete</button></div>');
  await page.addScriptTag({ content: runtimeSource });
  await page.addScriptTag({ content: takeoverSource });
  await page.locator('#delete').focus();
  const binding = await targetBinding(page, '#delete');
  expect(await page.evaluate(({ binding, key }) => (
    window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }
  ).__NEXA_TRUSTED_INPUT_GUARD__.arm('playwright-takeover-token', 'key-recycle', { pointerDown: 0, keyDown: 1, input: 0 }, { kind: 'key', key: key === 'Space' ? ' ' : key, ...binding }), { binding, key })).toBe(true);
  await page.keyboard.press(key);
  expect(await page.evaluate(() => (window as Window & { deleted?: boolean }).deleted)).toBeUndefined();
});
}

test('trusted Enter input keeps the target editor value outside its context fence', async ({ page }) => {
  await page.setContent('<!doctype html><div role="row"><span>Account A</span><textarea id="editor">Text</textarea></div>');
  await page.addScriptTag({ content: runtimeSource });
  await page.addScriptTag({ content: takeoverSource });
  await page.locator('#editor').focus();
  const binding = await targetBinding(page, '#editor');
  expect(await page.evaluate(binding => (
    window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }
  ).__NEXA_TRUSTED_INPUT_GUARD__.arm('playwright-takeover-token', 'editor-enter', { pointerDown: 0, keyDown: 1, input: 1 }, { kind: 'key', key: 'Enter', ...binding }), binding)).toBe(true);
  await page.keyboard.press('Enter');
  await expect(page.locator('#editor')).toHaveValue('\nText');
  expect(await page.evaluate(() => (window as unknown as {
    __NEXA_TRUSTED_INPUT_GUARD__: { disarm(token: string, id: string): boolean }
  }).__NEXA_TRUSTED_INPUT_GUARD__.disarm('playwright-takeover-token', 'editor-enter'))).toBe(true);
});

test('Agent drag uses observation-scoped endpoints and takeover removes its visual cursor', async ({ page }) => {
  await page.setContent(`
    <!doctype html>
    <button id="source" draggable="true">Move card</button>
    <div id="destination" class="drop-zone">Drop zone</div>
    <script>
      window.actionEvents = [];
      for (const type of ['pointermove', 'mousemove', 'dragstart', 'dragenter', 'dragover', 'drop', 'dragend']) {
        document.addEventListener(type, event => { window.actionEvents.push({ type, target: event.target.id, buttons: event.buttons }); event.preventDefault(); }, true);
      }
    </script>
  `);
  await page.addScriptTag({ content: runtimeSource });

  const observation = await observe(page);
  const sourceRef = observation.elements.find(element => element.name === 'Move card')!.ref;
  const endRef = observation.elements.find(element => element.name === 'Drop zone')!.ref;
  const input = actionInput(observation, 'drag', sourceRef, endRef);
  const preview = await page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.previewAction(value), input);
  await page.waitForTimeout(preview.durationMs + 20);
  await page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.act(value), input);

  const dragEvents = await page.evaluate(() => (
    window as unknown as { actionEvents: Array<{ type: string; buttons: number }> }
  ).actionEvents);
  expect(dragEvents.filter(event => event.type.startsWith('drag') || event.type === 'drop').map(event => event.type)).toEqual([
    'dragstart', 'dragenter', 'dragover', 'drop', 'dragend',
  ]);
  const pointerMoves = dragEvents.filter(event => event.type === 'pointermove' && event.buttons === 1);
  expect(pointerMoves).toHaveLength(8);
  expect(dragEvents.some(event => event.type === 'pointermove' && event.buttons === 0)).toBe(true);
  await page.evaluate(() => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.invalidateForUserTakeover());
  await expect(page.locator('[data-nexa-agent-cursor]')).toHaveCount(0);
});

test('Agent pointer preparation scrolls targets into view and rejects covered elements', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 600 });
  await page.setContent('<!doctype html><button id="target" style="margin-top:1800px">Far target</button>');
  await page.addScriptTag({ content: runtimeSource });

  const observation = await observe(page);
  const targetRef = observation.elements.find(element => element.name === 'Far target')!.ref;
  const prepared = await page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.prepareNativePointer(value), actionInput(observation, 'hover', targetRef));
  expect(await page.evaluate(() => scrollY)).toBeGreaterThan(0);
  expect(prepared.bounds.y).toBeGreaterThanOrEqual(0);
  expect(prepared.bounds.y + prepared.bounds.height).toBeLessThanOrEqual(600);

  await page.setContent(`
    <!doctype html>
    <button id="covered">Covered target</button>
    <div style="position:fixed;inset:0;z-index:10;background:white">Overlay</div>
  `);
  await page.addScriptTag({ content: runtimeSource });
  const coveredObservation = await observe(page);
  const coveredRef = coveredObservation.elements.find(element => element.name === 'Covered target')!.ref;
  await expect(page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.prepareNativePointer(value), actionInput(coveredObservation, 'hover', coveredRef)))
    .rejects.toThrow(/covered by another element/);
});

test('pointer preparation establishes the post-scroll baseline for noop effect verification', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 600 });
  await page.setContent('<!doctype html><button id="target" style="margin-top:1800px">Far target</button>');
  await page.addScriptTag({ content: runtimeSource });

  const beforePreparation = await observe(page);
  const targetRef = beforePreparation.elements.find(element => element.name === 'Far target')!.ref;
  const prepared = await page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.prepareNativePointer(value), actionInput(beforePreparation, 'hover', targetRef));
  const afterPreparation = await observe(page);

  expect(afterPreparation.domFingerprint).not.toBe(beforePreparation.domFingerprint);
  expect(prepared.verificationBaseline).toEqual({
    url: afterPreparation.url,
    domFingerprint: afterPreparation.domFingerprint,
    userEpoch: afterPreparation.userEpoch,
    observationOptions: { query: '', offset: 0 },
  });

  const bounds = await page.locator('#target').boundingBox();
  if (!bounds) throw new Error('Expected prepared browser target bounds');
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
  await page.locator('#target').evaluate(element => (element as HTMLButtonElement).click());
  const afterNoopActions = await observe(page);

  expect(snapshotChanged(prepared.verificationBaseline, afterNoopActions)).toBe(false);
});

test('same-length interactive attribute changes invalidate an observation', async ({ page }) => {
  await page.setContent('<!doctype html><a id="target" href="https://example.com/a">Open details</a>');
  await page.addScriptTag({ content: runtimeSource });
  const observation = await observe(page);
  const targetRef = observation.elements.find(element => element.name === 'Open details')!.ref;

  await page.locator('#target').evaluate((element) => {
    element.setAttribute('href', 'https://example.com/b');
  });

  await expect(page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.previewAction(value), actionInput(observation, 'click', targetRef)))
    .rejects.toThrow(/interactive page state changed/);
});

test('trusted input preparation validates, focuses, and selects editable targets', async ({ page }) => {
  await page.setContent('<!doctype html><input id="target" value="replace me"><button id="other">Other</button>');
  await page.addScriptTag({ content: runtimeSource });
  const observation = await observe(page);
  const targetRef = observation.elements.find(element => element.name === '')!.ref;
  const input = actionInput(observation, 'type', targetRef);

  const prepared = await page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.prepareTrustedText(value), input);
  expect(prepared.focused).toBe(true);
  expect(await page.locator('#target').evaluate((element) => (
    element as HTMLInputElement
  ).selectionStart)).toBe(0);
  expect(await page.locator('#target').evaluate((element) => (
    element as HTMLInputElement
  ).selectionEnd)).toBe('replace me'.length);

  const next = await observe(page);
  const otherRef = next.elements.find(element => element.name === 'Other')!.ref;
  await expect(page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.prepareTrustedText(value), actionInput(next, 'type', otherRef)))
    .rejects.toThrow(/editable target/);
});

test('trusted key preparation verifies focus in a same-origin iframe document', async ({ page }) => {
  await page.setContent(`
    <!doctype html>
    <iframe id="frame" srcdoc="<!doctype html><button id='target'>Frame action</button>"></iframe>
  `);
  await expect(page.frameLocator('#frame').locator('#target')).toBeVisible();
  await page.addScriptTag({ content: runtimeSource });
  const observation = await observe(page);
  const targetRef = observation.elements.find(element => element.name === 'Frame action')?.ref;
  expect(targetRef).toBeTruthy();

  const prepared = await page.evaluate(value => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.prepareTrustedKey(value), actionInput(observation, 'press', targetRef!));

  expect(prepared.focused).toBe(true);
  await expect(page.frameLocator('#frame').locator('#target')).toBeFocused();
  expect(await page.locator('#frame').evaluate(element => (
    element.ownerDocument.activeElement === element
  ))).toBe(true);
});

test('authenticated trusted input does not masquerade as user takeover state', async ({ page }) => {
  await page.setContent('<!doctype html><button id="target">Open details</button>');
  await page.addScriptTag({ content: runtimeSource });
  await page.addScriptTag({ content: takeoverSource });
  const before = await observe(page);
  const targetBounds = await page.locator('#target').boundingBox();
  if (!targetBounds) throw new Error('Expected trusted input target bounds');
  const expectedPointer = {
    ...await targetBinding(page, '#target'),
    kind: 'pointer' as const,
    x: targetBounds.x + targetBounds.width / 2,
    y: targetBounds.y + targetBounds.height / 2,
    button: 'left',
  };

  expect(await page.evaluate((expected) => (
    window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }
  ).__NEXA_TRUSTED_INPUT_GUARD__.arm(
    'playwright-takeover-token',
    'operation-1',
    { pointerDown: 1, keyDown: 0, input: 0 },
    expected,
  ), expectedPointer)).toBe(true);
  await page.locator('#target').click();
  const guarded = await observe(page);
  expect(guarded.userEpoch).toBe(before.userEpoch);

  expect(await page.evaluate(({ expected }) => (
    window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }
  ).__NEXA_TRUSTED_INPUT_GUARD__.arm(
    'playwright-takeover-token',
    'operation-2',
    { pointerDown: 1, keyDown: 0, input: 0 },
    { ...expected, x: expected.x + 100 },
  ), { expected: expectedPointer })).toBe(false);
  await page.locator('#target').click();
  const unguarded = await observe(page);
  expect(unguarded.userEpoch).toBe(before.userEpoch + 1);
});

test('trusted pointer guard rejects a replacement element at the approved coordinates', async ({ page }) => {
  await page.setContent('<!doctype html><button id="target">Approved target</button>');
  await page.addScriptTag({ content: runtimeSource });
  await page.addScriptTag({ content: takeoverSource });
  const before = await observe(page);
  const targetBounds = await page.locator('#target').boundingBox();
  if (!targetBounds) throw new Error('Expected trusted input target bounds');
  const expected = {
    ...await targetBinding(page, '#target'),
    kind: 'pointer' as const,
    x: targetBounds.x + targetBounds.width / 2,
    y: targetBounds.y + targetBounds.height / 2,
    button: 'left' as const,
  };

  expect(await page.evaluate((pointer) => (
    window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }
  ).__NEXA_TRUSTED_INPUT_GUARD__.arm(
    'playwright-takeover-token',
    'replacement-operation',
    { pointerDown: 1, keyDown: 0, input: 0 },
    pointer,
  ), expected)).toBe(true);
  await page.evaluate(() => {
    const original = document.querySelector('#target');
    const replacement = document.createElement('button');
    replacement.id = 'replacement';
    replacement.textContent = 'Different action';
    original?.replaceWith(replacement);
  });
  await page.locator('#replacement').click();

  expect((await observe(page)).userEpoch).toBe(before.userEpoch + 1);
});

test('trusted text and key guards require the exact dispatched event signature', async ({ page }) => {
  await page.setContent('<!doctype html><input id="target"><button id="button">Continue</button>');
  await page.addScriptTag({ content: runtimeSource });
  await page.addScriptTag({ content: takeoverSource });
  await page.locator('#target').focus();
  const beforeText = await observe(page);

  expect(await page.evaluate((binding) => (
    window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }
  ).__NEXA_TRUSTED_INPUT_GUARD__.arm(
    'playwright-takeover-token',
    'text-operation',
    { pointerDown: 0, keyDown: 0, input: 1 },
    { kind: 'text', data: 'agent text', ...binding },
  ), await targetBinding(page, '#target'))).toBe(true);
  await page.keyboard.insertText('agent text');
  expect((await observe(page)).userEpoch).toBe(beforeText.userEpoch);

  await page.locator('#button').focus();
  const beforeKey = await observe(page);
  expect(await page.evaluate((binding) => (
    window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }
  ).__NEXA_TRUSTED_INPUT_GUARD__.arm(
    'playwright-takeover-token',
    'key-operation',
    { pointerDown: 0, keyDown: 1, input: 0 },
    { kind: 'key', key: 'Escape', ...binding },
  ), await targetBinding(page, '#button'))).toBe(true);
  await page.keyboard.press('Escape');
  expect((await observe(page)).userEpoch).toBe(beforeKey.userEpoch);

  expect(await page.evaluate((binding) => (
    window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }
  ).__NEXA_TRUSTED_INPUT_GUARD__.arm(
    'playwright-takeover-token',
    'mismatched-text-operation',
    { pointerDown: 0, keyDown: 0, input: 1 },
    { kind: 'text', data: 'agent text', ...binding },
  ), await targetBinding(page, '#button'))).toBe(true);
  await page.locator('#target').focus();
  await page.keyboard.insertText('user text');
  expect((await observe(page)).userEpoch).toBe(beforeKey.userEpoch + 1);
});

test('form observations expose labels, checkbox state and bounded select choices', async ({ page }) => {
  await page.setContent(`<label for="choice">Remember choice</label><input id="choice" type="checkbox" checked>
    <div role="switch" aria-label="Notifications" aria-checked="mixed" aria-disabled="true">Toggle</div>
    <label>Country<select><option value="cn" selected>China</option><option value="de" disabled>Germany</option></select></label>`);
  await page.addScriptTag({ content: runtimeSource });
  const snapshot = await observe(page);
  expect(snapshot.elements.find(el => el.name === 'Remember choice')).toMatchObject({ role: 'checkbox', checked: true });
  expect(snapshot.elements.find(el => el.name === 'Notifications')).toMatchObject({ role: 'switch', checked: 'mixed', enabled: false });
  expect(snapshot.elements.find(el => el.role === 'combobox')).toMatchObject({
    options: [{ value: 'cn', label: 'China', selected: true, enabled: true }, { value: 'de', label: 'Germany', selected: false, enabled: false }],
  });
});

test('set_checked changes once and repeated desired state does not click again', async ({ page }) => {
  await page.setContent('<input type="checkbox" aria-label="Remember" onclick="window.clicks=(window.clicks||0)+1">');
  await page.addScriptTag({ content: runtimeSource });
  for (const checked of [true, true, false, false]) {
    const snapshot = await observe(page);
    const input = { ...actionInput(snapshot, 'set_checked', snapshot.elements[0].ref), checked };
    await page.evaluate(input => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.act(input), input);
    await expect(page.locator('input')).toBeChecked({ checked });
    const fresh = await observe(page);
    const prepared = await page.evaluate(input => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.prepareNativePointer(input),
      { ...actionInput(fresh, 'set_checked', fresh.elements[0].ref), checked });
    expect(prepared.stateMatched).toBe(true);
  }
  expect(await page.evaluate(() => (window as Window & { clicks: number }).clicks)).toBe(2);
});

test('set_checked rejects disabled controls, radio clearing and stale checked state', async ({ page }) => {
  await page.setContent('<input type="checkbox" disabled aria-label="Locked"><input type="radio" checked aria-label="Plan"><input type="checkbox" aria-label="Changed">');
  await page.addScriptTag({ content: runtimeSource });
  for (const [name, checked] of [['Locked', true], ['Plan', false]] as const) {
    const snapshot = await observe(page);
    const input = { ...actionInput(snapshot, 'set_checked', snapshot.elements.find(el => el.name === name)!.ref), checked };
    await expect(page.evaluate(input => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.act(input), input)).rejects.toThrow();
  }
  const old = await observe(page);
  await page.locator('[aria-label="Changed"]').evaluate(el => { (el as HTMLInputElement).checked = true; });
  await expect(page.evaluate(input => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.act(input),
    { ...actionInput(old, 'set_checked', old.elements.find(el => el.name === 'Changed')!.ref), checked: false })).rejects.toThrow('stale observation');
});

test('set_checked preparation uses guarded trusted clicks and skips matching state', async ({ page }) => {
  await page.setContent('<input type="checkbox" aria-label="Remember" onchange="window.trusted=event.isTrusted">');
  await page.addScriptTag({ content: runtimeSource });
  await page.addScriptTag({ content: takeoverSource });
  const snapshot = await observe(page);
  const input = { ...actionInput(snapshot, 'set_checked', snapshot.elements[0].ref), checked: true };
  const prepared = await page.evaluate(input => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.prepareNativePointer(input), input);
  const binding = await targetBinding(page, 'input');
  const x = prepared.bounds.x + prepared.bounds.width / 2;
  const y = prepared.bounds.y + prepared.bounds.height / 2;
  expect(await page.evaluate(({ binding, x, y }) => (window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }).__NEXA_TRUSTED_INPUT_GUARD__.arm(
    'playwright-takeover-token', 'checked-click', { pointerDown: 1, keyDown: 0, input: 1 }, { kind: 'pointer', x, y, button: 'left', ...binding }), { binding, x, y })).toBe(true);
  await page.mouse.click(x, y);
  expect(await page.evaluate(() => (window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }).__NEXA_TRUSTED_INPUT_GUARD__.disarm('playwright-takeover-token', 'checked-click'))).toBe(true);
  await expect(page.locator('input')).toBeChecked();
  expect(await page.evaluate(() => (window as Window & { trusted: boolean }).trusted)).toBe(true);
  const fresh = await observe(page);
  expect((await page.evaluate(input => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.prepareNativePointer(input),
    { ...actionInput(fresh, 'set_checked', fresh.elements[0].ref), checked: true })).stateMatched).toBe(true);
});

test('large forms bound total option evidence and report omitted choices', async ({ page }) => {
  await page.setContent(Array.from({ length: 8 }, (_, index) => `<select aria-label="Select ${index}">${Array.from({ length: 150 }, (_, option) => `<option value="${option}">Choice ${option}</option>`).join('')}</select>`).join(''));
  await page.addScriptTag({ content: runtimeSource });
  const snapshot = await observe(page);
  expect(snapshot.elements).toHaveLength(8);
  expect(snapshot.elements.reduce((total, el) => total + (el.options?.length || 0), 0)).toBeLessThanOrEqual(400);
  expect(snapshot.elements.every(el => el.optionCount === 150)).toBe(true);
  expect(snapshot.elements.some(el => (el.options?.length || 0) < el.optionCount!)).toBe(true);
});

test('select rejects missing and disabled choices without clearing the current value', async ({ page }) => {
  await page.setContent('<select aria-label="Plan"><option value="free" selected>Free</option><optgroup disabled><option value="paid">Paid</option></optgroup></select>');
  await page.addScriptTag({ content: runtimeSource });
  for (const value of ['missing', 'paid']) {
    const snapshot = await observe(page);
    await expect(page.evaluate(input => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.act(input),
      { ...actionInput(snapshot, 'select', snapshot.elements[0].ref), value })).rejects.toThrow();
    await expect(page.locator('select')).toHaveValue('free');
  }
});

test('select supports exact multiple selections and skips unchanged values', async ({ page }) => {
  await page.setContent('<select multiple aria-label="Tags" onchange="window.changes=(window.changes||0)+1"><option value="a">A</option><option value="b">B</option><option value="c">C</option></select>');
  await page.addScriptTag({ content: runtimeSource });
  for (const values of [['a', 'c'], ['a', 'c'], []]) {
    const snapshot = await observe(page);
    await page.evaluate(input => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.act(input),
      { ...actionInput(snapshot, 'select', snapshot.elements[0].ref), values });
    await expect(page.locator('select')).toHaveValues(values);
  }
  expect(await page.evaluate(() => (window as Window & { changes: number }).changes)).toBe(2);
});

test('observations distinguish loading documents from complete documents', async ({ page }) => {
  await page.setContent('<button>Ready</button>');
  await page.addScriptTag({ content: runtimeSource });
  await page.evaluate(() => Object.defineProperty(document, 'readyState', { configurable: true, get: () => 'loading' }));
  expect((await observe(page)).readyState).toBe('loading');
  await page.evaluate(() => Object.defineProperty(document, 'readyState', { configurable: true, get: () => 'complete' }));
  expect((await observe(page)).readyState).toBe('complete');
});

test('hidden file inputs support an exact guarded native file selection', async ({ page }, testInfo) => {
  const path = testInfo.outputPath('proof.txt');
  const content = 'Nexa upload fixture 中文';
  mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, content);
  await page.setContent('<input id="upload" type="file" hidden aria-label="Upload evidence">');
  await page.addScriptTag({ content: runtimeSource });
  await page.addScriptTag({ content: takeoverSource });
  const snapshot = await observe(page);
  const target = snapshot.elements.find(el => el.name === 'Upload evidence');
  expect(target).toBeTruthy();
  const files = [{ name: 'proof.txt', size: Buffer.byteLength(content) }];
  const input = { ...actionInput(snapshot, 'upload_files', target!.ref), files };
  const prepared = await page.evaluate(input => (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__.prepareTrustedUpload(input), input);
  expect(await page.evaluate(({ prepared, files }) => (window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }).__NEXA_TRUSTED_INPUT_GUARD__.arm(
    'playwright-takeover-token', 'upload', { pointerDown: 0, keyDown: 0, input: 1 }, { kind: 'files', files, targetRef: prepared.targetRef, targetContext: prepared.targetContext }), { prepared, files })).toBe(true);
  const cdp = await page.context().newCDPSession(page);
  const object = await cdp.send('Runtime.evaluate', { expression: 'document.getElementById("upload")', returnByValue: false });
  await cdp.send('DOM.setFileInputFiles', { files: [path], objectId: object.result.objectId });
  expect(await page.evaluate(() => (window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }).__NEXA_TRUSTED_INPUT_GUARD__.disarm('playwright-takeover-token', 'upload'))).toBe(true);
  const after = await observe(page);
  expect(after.userEpoch).toBe(snapshot.userEpoch);
  expect(after.elements.find(el => el.ref === target!.ref)).toMatchObject({ fileCount: 1, files });
  expect(await page.locator('input').evaluate(el => (el as HTMLInputElement).files![0].text())).toBe(content);
  await cdp.detach();
});

async function observe(page: import('@playwright/test').Page): Promise<BrowserObservation> {
  return page.evaluate(() => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.observe());
}

async function installShadowControls(page: import('@playwright/test').Page) {
  await page.setContent('<!doctype html><main id="host"></main><button id="outside">Outside</button>');
  await page.evaluate(() => {
    const root = document.getElementById('host')!.attachShadow({ mode: 'open' });
    root.innerHTML = '<section id="nested"></section>';
    const nested = root.querySelector('#nested')!.attachShadow({ mode: 'open' });
    nested.innerHTML = '<button id="action">Component action</button><input aria-label="Component editor" value="old"><input type="checkbox" aria-label="Component toggle">';
    nested.querySelector('button')!.addEventListener('click', event => {
      (event.currentTarget as HTMLButtonElement).textContent = 'Component completed';
    });
  });
  await page.addScriptTag({ content: runtimeSource });
  await page.addScriptTag({ content: takeoverSource });
}

test('native pointer preparation and trusted clicks reach nested web component controls', async ({ page }) => {
  await installShadowControls(page);
  const observation = await observe(page);
  const target = observation.elements.find(element => element.name === 'Component action')!;
  const prepared = await page.evaluate(input => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.prepareNativePointer(input), actionInput(observation, 'click', target.ref));
  const x = prepared.bounds.x + prepared.bounds.width / 2;
  const y = prepared.bounds.y + prepared.bounds.height / 2;
  const armed = await page.evaluate(({ targetRef, x, y }) => {
    const fixture = window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge; __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard };
    const target = fixture.__NEXA_BROWSER_RUNTIME__.resolveTargetRef(targetRef)!;
    return fixture.__NEXA_TRUSTED_INPUT_GUARD__.arm('playwright-takeover-token', 'shadow-click', { pointerDown: 1, keyDown: 0, input: 0 }, {
      kind: 'pointer', x, y, button: 'left', targetRef, targetContext: fixture.__NEXA_BROWSER_RUNTIME__.targetContextFingerprint(target),
    });
  }, { targetRef: target.ref, x, y });
  expect(armed).toBe(true);
  await page.mouse.click(x, y);
  await expect(page.getByRole('button', { name: 'Component completed' })).toBeVisible();
  expect((await observe(page)).userEpoch).toBe(observation.userEpoch);
});

test('trusted text input reaches nested web component editors without user takeover', async ({ page }) => {
  await installShadowControls(page);
  const observation = await observe(page);
  const target = observation.elements.find(element => element.name === 'Component editor')!;
  const prepared = await page.evaluate(input => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.prepareTrustedText(input), actionInput(observation, 'type', target.ref));
  expect(prepared.focused).toBe(true);
  expect(await page.evaluate(targetRef => {
    const fixture = window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge; __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard };
    const target = fixture.__NEXA_BROWSER_RUNTIME__.resolveTargetRef(targetRef)!;
    return fixture.__NEXA_TRUSTED_INPUT_GUARD__.arm('playwright-takeover-token', 'shadow-text', { pointerDown: 0, keyDown: 0, input: 1 }, {
      kind: 'text', data: '组件 editor', targetRef, targetContext: fixture.__NEXA_BROWSER_RUNTIME__.targetContextFingerprint(target),
    });
  }, target.ref)).toBe(true);
  await page.keyboard.insertText('组件 editor');
  await expect(page.getByRole('textbox', { name: 'Component editor' })).toHaveValue('组件 editor');
  expect((await observe(page)).userEpoch).toBe(observation.userEpoch);
});

test('trusted keyboard activation reaches web components and still detects unrelated user input', async ({ page }) => {
  await installShadowControls(page);
  const observation = await observe(page);
  const target = observation.elements.find(element => element.name === 'Component toggle')!;
  const prepared = await page.evaluate(input => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.prepareTrustedKey(input), actionInput(observation, 'press', target.ref));
  expect(prepared.focused).toBe(true);
  expect(await page.evaluate(targetRef => {
    const fixture = window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge; __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard };
    const target = fixture.__NEXA_BROWSER_RUNTIME__.resolveTargetRef(targetRef)!;
    return fixture.__NEXA_TRUSTED_INPUT_GUARD__.arm('playwright-takeover-token', 'shadow-key', { pointerDown: 0, keyDown: 1, input: 1 }, {
      kind: 'key', key: ' ', targetRef, targetContext: fixture.__NEXA_BROWSER_RUNTIME__.targetContextFingerprint(target),
    });
  }, target.ref)).toBe(true);
  await page.keyboard.press('Space');
  await expect(page.getByRole('checkbox', { name: 'Component toggle' })).toBeChecked();
  expect((await observe(page)).userEpoch).toBe(observation.userEpoch);
  await page.getByRole('button', { name: 'Outside', exact: true }).click();
  expect((await observe(page)).userEpoch).toBeGreaterThan(observation.userEpoch);
});

test('web component pointer preparation rejects an overlay outside its shadow root', async ({ page }) => {
  await installShadowControls(page);
  await page.evaluate(() => {
    const overlay = document.createElement('div');
    overlay.style.cssText = 'position:fixed;inset:0;z-index:999;background:white';
    document.body.append(overlay);
  });
  const observation = await observe(page);
  const target = observation.elements.find(element => element.name === 'Component action')!;
  await expect(page.evaluate(input => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.prepareNativePointer(input), actionInput(observation, 'click', target.ref))).rejects.toThrow(/covered by another element/);
});

test('web component controls respect inert shadow hosts', async ({ page }) => {
  await installShadowControls(page);
  await page.locator('#host').evaluate(host => { host.inert = true; });
  const observation = await observe(page);
  const target = observation.elements.find(element => element.name === 'Component action')!;
  await expect(page.evaluate(input => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.prepareNativePointer(input), actionInput(observation, 'click', target.ref))).rejects.toThrow(/disabled/);
});

test('trusted web component actions reject an enclosing row recycled after arming', async ({ page }) => {
  await installShadowControls(page);
  await page.evaluate(() => {
    const row = document.createElement('article');
    row.innerHTML = '<span id="identity">Account A</span>';
    const host = document.getElementById('host')!;
    host.before(row);
    row.append(host);
  });
  const observation = await observe(page);
  const target = observation.elements.find(element => element.name === 'Component action')!;
  const prepared = await page.evaluate(input => (
    window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }
  ).__NEXA_BROWSER_RUNTIME__.prepareNativePointer(input), actionInput(observation, 'click', target.ref));
  const x = prepared.bounds.x + prepared.bounds.width / 2;
  const y = prepared.bounds.y + prepared.bounds.height / 2;
  expect(await page.evaluate(({ targetRef, x, y }) => {
    const fixture = window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge; __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard };
    const element = fixture.__NEXA_BROWSER_RUNTIME__.resolveTargetRef(targetRef)!;
    return fixture.__NEXA_TRUSTED_INPUT_GUARD__.arm('playwright-takeover-token', 'shadow-recycle', { pointerDown: 1, keyDown: 0, input: 0 }, {
      kind: 'pointer', x, y, button: 'left', targetRef, targetContext: fixture.__NEXA_BROWSER_RUNTIME__.targetContextFingerprint(element),
    });
  }, { targetRef: target.ref, x, y })).toBe(true);
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.locator('#identity').evaluate(el => { el.textContent = 'Account B'; });
  await page.mouse.up();
  await expect(page.getByRole('button', { name: 'Component action', exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as {
    __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard;
  }).__NEXA_TRUSTED_INPUT_GUARD__.disarm('playwright-takeover-token', 'shadow-recycle'))).toBe(false);
});

async function targetBinding(page: import('@playwright/test').Page, selector: string) {
  return page.evaluate(selector => {
    const bridge = (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__;
    const element = document.querySelector(selector)!;
    const observation = bridge.observe();
    return {
      targetRef: observation.elements.find(item => bridge.resolveTargetRef(item.ref) === element)!.ref,
      targetContext: bridge.targetContextFingerprint(element),
    };
  }, selector);
}

function actionInput(
  observation: BrowserObservation,
  action: string,
  targetRef: string,
  endRef?: string,
): BrowserActionInput {
  return {
    action,
    targetRef,
    endRef,
    button: 'left',
    modifiers: [],
    userEpoch: observation.userEpoch,
    domFingerprint: observation.domFingerprint,
    interactionFingerprint: observation.interactionFingerprint,
    expected: observation.elements.find(element => element.ref === targetRef),
    expectedEnd: observation.elements.find(element => element.ref === endRef),
  };
}

interface BrowserElement {
  ref: string;
  name: string;
  role: string;
  checked?: boolean | 'mixed';
  options?: unknown[];
  optionCount?: number;
  bounds: { x: number; y: number; width: number; height: number };
}

interface BrowserObservation {
  url: string;
  readyState?: string;
  userEpoch: number;
  domFingerprint: string;
  interactionFingerprint: string;
  elements: BrowserElement[];
}

interface BrowserVerificationBaseline {
  url: string;
  userEpoch: number;
  domFingerprint: string;
}

interface BrowserActionInput {
  action: string;
  point?: { x: number; y: number };
  endPoint?: { x: number; y: number };
  presentationId?: string;
  checked?: boolean;
  value?: string;
  values?: string[];
  files?: Array<{ name: string; size: number }>;
  targetRef: string;
  endRef?: string;
  button: string;
  modifiers: string[];
  userEpoch: number;
  domFingerprint: string;
  interactionFingerprint: string;
  expected?: BrowserElement;
  expectedEnd?: BrowserElement;
}

test('Screenshot coordinates bind the exact hit target and retain link approval metadata', async ({ page }) => {
  await page.setContent('<body style="margin:0"><a href="https://example.com/details" style="display:block;width:300px;height:100px"><span>Details</span></a><canvas width="300" height="150"></canvas></body>');
  await page.addScriptTag({ content: runtimeSource });
  const result = await page.evaluate(() => {
    const bridge = (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__;
    const observation = bridge.observe();
    const input = { ...observation, action: 'click', targetRef: '', button: 'left', modifiers: [] };
    const link = bridge.resolvePointerTarget(input, { x: 20, y: 10 }) as BrowserElement & { href: string };
    const point = { x: 25, y: 125 };
    const canvas = bridge.resolvePointerTarget(input, point);
    const prepared = bridge.prepareNativePointer({ ...input, point, targetRef: canvas.ref, expected: canvas });
    const failures = [{ x: -1, y: 20 }, { x: innerWidth, y: 10 }, { x: NaN, y: 20 }].map(point => {
      try { bridge.resolvePointerTarget(input, point); return false; } catch { return true; }
    });
    document.querySelector('canvas')!.replaceWith(document.querySelector('canvas')!.cloneNode());
    try { bridge.resolvePointerTarget(input, point); failures.push(false); } catch { failures.push(true); }
    bridge.invalidateForUserTakeover();
    try { bridge.resolvePointerTarget(input, point); failures.push(false); } catch { failures.push(true); }
    return { link, prepared, failures };
  });
  expect(result.link.href).toBe('https://example.com/details');
  expect(result.prepared.bounds).toEqual({ x: 25, y: 125, width: 0, height: 0 });
  expect(result.failures).toEqual([true, true, true, true, true]);
});

test('Native drag also delivers trusted HTML drop events', async ({ page }) => {
  await page.setContent('<div draggable="true" style="position:absolute;left:20px;top:20px;width:80px;height:80px" ondragstart="event.dataTransfer.setData(\'text/plain\',\'fixture\')">Source</div><div data-dropzone style="position:absolute;left:220px;top:20px;width:100px;height:100px" ondragover="event.preventDefault()" ondrop="event.preventDefault();window.dropped=event.isTrusted&&event.dataTransfer.getData(\'text/plain\')===\'fixture\'">Destination</div>');
  await page.addScriptTag({ content: runtimeSource });
  await page.addScriptTag({ content: takeoverSource });
  const prepared = await page.evaluate(() => {
    const bridge = (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__;
    const observation = bridge.observe();
    const target = observation.elements.find(el => el.name === 'Source')!, end = observation.elements.find(el => el.name === 'Destination')!;
    return bridge.prepareTrustedDrag({ ...observation, action: 'drag', targetRef: target.ref, endRef: end.ref, expected: target, expectedEnd: end, button: 'left', modifiers: [] });
  });
  expect(await page.evaluate(prepared => (window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }).__NEXA_TRUSTED_INPUT_GUARD__.arm('playwright-takeover-token', 'html-drag', { pointerDown: 1, keyDown: 0, input: 0 }, { kind: 'drag', x: 60, y: 60, endX: 270, endY: 70, button: 'left', ...prepared }), prepared)).toBe(true);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 60, y: 60 });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: 60, y: 60, button: 'left', buttons: 1, clickCount: 1 });
  for (let step = 1; step <= 16; step++) await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 60 + 210 * step / 16, y: 60 + 10 * step / 16, button: 'left', buttons: 1 });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 270, y: 70, button: 'left', buttons: 0, clickCount: 1 });
  expect(await page.evaluate(() => (window as unknown as { dropped: boolean }).dropped)).toBe(true);
  expect(await page.evaluate(() => (window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }).__NEXA_TRUSTED_INPUT_GUARD__.disarm('playwright-takeover-token', 'html-drag'))).toBe(true);
});

test('Trusted wheel reaches the nested scroll area without surrendering browser control', async ({ page }) => {
  await page.setContent('<body style="margin:0;height:2000px"><div style="height:200px;width:300px;overflow:auto"><div style="height:1500px">Nested content</div></div></body>');
  await page.addScriptTag({ content: runtimeSource });
  await page.addScriptTag({ content: takeoverSource });
  const binding = await page.evaluate(() => {
    const bridge = (window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge }).__NEXA_BROWSER_RUNTIME__;
    const observation = bridge.observe();
    const input = { ...observation, action: 'scroll', targetRef: '', point: { x: 100, y: 100 }, button: 'left', modifiers: [] };
    const target = bridge.resolvePointerTarget(input, input.point);
    const prepared = bridge.prepareNativePointer({ ...input, targetRef: target.ref, expected: target });
    return { targetRef: prepared.targetRef, targetContext: prepared.targetContext, userEpoch: observation.userEpoch };
  });
  expect(await page.evaluate(binding => (window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }).__NEXA_TRUSTED_INPUT_GUARD__.arm('playwright-takeover-token', 'wheel', { pointerDown: 0, keyDown: 0, input: 0, wheel: 1 }, { kind: 'wheel', x: 100, y: 100, deltaX: 0, deltaY: 240, ...binding }), binding)).toBe(true);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: 100, y: 100, deltaX: 0, deltaY: 240 });
  await expect.poll(() => page.locator('body > div').evaluate(el => el.scrollTop)).toBeGreaterThan(0);
  expect(await page.evaluate(() => scrollY)).toBe(0);
  expect((await observe(page)).userEpoch).toBe(binding.userEpoch);
  expect(await page.evaluate(() => (window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }).__NEXA_TRUSTED_INPUT_GUARD__.disarm('playwright-takeover-token', 'wheel'))).toBe(true);
});

test('Trusted drag delivers real held-button pointer events to a canvas', async ({ page }) => {
  await page.setContent('<body style="margin:0"><canvas width="500" height="300"></canvas></body>');
  await page.addScriptTag({ content: runtimeSource });
  await page.addScriptTag({ content: takeoverSource });
  const prepared = await page.evaluate(() => {
    const fixture = window as unknown as { __NEXA_BROWSER_RUNTIME__: BrowserBridge; dragEvents: Array<{ type: string; trusted: boolean; buttons: number }> };
    fixture.dragEvents = [];
    for (const type of ['pointerdown', 'pointermove', 'pointerup']) document.querySelector('canvas')!.addEventListener(type, event => fixture.dragEvents.push({ type, trusted: event.isTrusted, buttons: (event as PointerEvent).buttons }));
    const bridge = fixture.__NEXA_BROWSER_RUNTIME__;
    const observation = bridge.observe();
    const input = { ...observation, action: 'drag', targetRef: '', point: { x: 30, y: 40 }, endPoint: { x: 250, y: 160 }, button: 'left', modifiers: [] };
    const start = bridge.resolvePointerTarget(input, input.point);
    const end = bridge.resolvePointerTarget(input, input.endPoint);
    return { ...bridge.prepareTrustedDrag({ ...input, targetRef: start.ref, expected: start, endRef: end.ref, expectedEnd: end }), userEpoch: observation.userEpoch };
  });
  expect(await page.evaluate(prepared => (window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }).__NEXA_TRUSTED_INPUT_GUARD__.arm('playwright-takeover-token', 'drag', { pointerDown: 1, keyDown: 0, input: 0 }, { kind: 'drag', x: 30, y: 40, endX: 250, endY: 160, button: 'left', ...prepared }), prepared)).toBe(true);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 30, y: 40 });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: 30, y: 40, button: 'left', buttons: 1, clickCount: 1 });
  for (let step = 1; step <= 8; step++) await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 30 + 220 * step / 8, y: 40 + 120 * step / 8, button: 'left', buttons: 1 });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 250, y: 160, button: 'left', buttons: 0, clickCount: 1 });
  const events = await page.evaluate(() => (window as unknown as { dragEvents: Array<{ type: string; trusted: boolean; buttons: number }> }).dragEvents);
  expect(events.filter(event => event.type === 'pointermove' && event.buttons === 1)).toHaveLength(8);
  expect(events.every(event => event.trusted)).toBe(true);
  expect(events.at(-1)?.type).toBe('pointerup');
  expect(await page.evaluate(() => (window as unknown as { __NEXA_TRUSTED_INPUT_GUARD__: TrustedInputGuard }).__NEXA_TRUSTED_INPUT_GUARD__.disarm('playwright-takeover-token', 'drag'))).toBe(true);
  expect((await observe(page)).userEpoch).toBe(prepared.userEpoch);
});

interface BrowserBridge {
  resolvePointerTarget(input: BrowserActionInput, point: { x: number; y: number }): BrowserElement;
  prepareTrustedDrag(input: BrowserActionInput): { targetRef: string; targetContext: string; endRef: string; endContext: string; bounds: { x: number; y: number; width: number; height: number }; endBounds: { x: number; y: number; width: number; height: number } };
  prepareTrustedUpload(input: BrowserActionInput): { targetRef: string; targetContext: string; verificationBaseline: BrowserVerificationBaseline };
  targetContextFingerprint(element: Element): string;
  resolveTargetRef(ref: string): Element | null;
  observe(): BrowserObservation;
  previewAction(input: BrowserActionInput): { durationMs: number; presentationId: string };
  presentationStatus(id: string): { id: string; status: string };
  cancelPresentation(id: string): boolean;
  beginCapture(id: string): { id: string };
  captureStatus(id: string): { id: string; status: string };
  endCapture(id: string): boolean;
  prepareNativePointer(input: BrowserActionInput): {
    targetRef: string;
    targetContext: string;
    stateMatched?: boolean;
    bounds: { x: number; y: number; width: number; height: number };
    verificationBaseline: BrowserVerificationBaseline;
  };
  prepareTrustedText(input: BrowserActionInput): {
    focused: boolean;
    verificationBaseline: BrowserVerificationBaseline;
  };
  prepareTrustedKey(input: BrowserActionInput): {
    focused: boolean;
    verificationBaseline: BrowserVerificationBaseline;
  };
  act(input: BrowserActionInput): { inputSent: boolean };
  invalidateForUserTakeover(): void;
}

function snapshotChanged(
  before: BrowserVerificationBaseline,
  after: BrowserObservation,
): boolean {
  return before.url !== after.url
    || before.domFingerprint !== after.domFingerprint
    || before.userEpoch !== after.userEpoch;
}

interface TrustedInputGuard {
  disarm(token: string, operationId: string): boolean;
  arm(
    token: string,
    operationId: string,
    budget: { pointerDown: number; keyDown: number; input: number; wheel?: number },
    expected: ({
      kind: 'pointer';
      x: number;
      y: number;
      button: 'left' | 'middle' | 'right';
    } | { kind: 'wheel'; x: number; y: number; deltaX: number; deltaY: number } | { kind: 'drag'; x: number; y: number; endX: number; endY: number; endRef: string; endContext: string; button: 'left' } | { kind: 'text'; data: string } | { kind: 'key'; key: string; modifiers?: number } | { kind: 'files'; files: Array<{ name: string; size: number }> }) & { targetRef: string; targetContext: string },
  ): boolean;
}
