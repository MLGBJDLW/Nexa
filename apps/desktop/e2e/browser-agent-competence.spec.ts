import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';

// Run the production observation script, not a replacement DOM extractor.
const source = readFileSync(join(process.cwd(), 'src-tauri', 'src', 'browser', 'scripts.rs'), 'utf8');
const runtimeSource = source.match(/pub const BROWSER_INIT_SCRIPT: &str = r#"\r?\n([\s\S]*?)\r?\n"#;/)?.[1]
  .replace('__NEXA_PICK_TOKEN__', JSON.stringify('competence-fixture-token'));
if (!runtimeSource) throw new Error('Could not extract Browser Workspace runtime');

type ObservedElement = {
  ref: string;
  name: string;
  role: string;
  value?: string | null;
  valueTruncated?: boolean | null;
  checked?: boolean;
  selectedValues?: string[];
  bounds: { x: number; y: number; width: number; height: number };
};
type ObservationOptions = { query?: string; offset?: number };
type Observation = {
  elements: ObservedElement[];
  userEpoch: number;
  interactionFingerprint: string;
  domFingerprint: string;
  observationCoverage: { query: string; offset: number; returned: number; totalMatches: number; hasMore: boolean; nextOffset: number | null };
  frameLimitations: { unavailableCount: number; detailsOmitted: boolean; frames: Array<{ reason: string; url: string | null; title: string; visible: boolean }> };
};

async function observe(page: Page, options: ObservationOptions = {}): Promise<Observation> {
  return page.evaluate(options => (window as unknown as {
    __NEXA_BROWSER_RUNTIME__: { observe(options: ObservationOptions): Observation };
  }).__NEXA_BROWSER_RUNTIME__.observe(options), options);
}

async function act(page: Page, observation: Observation, name: string, action: string, text?: string, extra: Record<string, unknown> = {}): Promise<void> {
  const expected = observation.elements.find(element => element.name === name);
  expect(expected, `Missing actionable ref for ${name}`).toBeTruthy();
  await page.evaluate(input => (window as unknown as {
    __NEXA_BROWSER_RUNTIME__: { act(input: unknown): void };
  }).__NEXA_BROWSER_RUNTIME__.act(input), {
    userEpoch: observation.userEpoch,
    interactionFingerprint: observation.interactionFingerprint,
    targetRef: expected!.ref,
    expected,
    action,
    text,
    ...extra,
  });
}

test('agent can distinguish form controls labelled through aria-labelledby', async ({ page }) => {
  await page.setContent(`<!doctype html>
    <span id="billing">Billing</span><span id="shipping">Shipping</span><span id="address">address</span>
    <input aria-labelledby="billing address" value="Original billing">
    <input aria-labelledby="shipping address" value="Original shipping">`);
  await page.addScriptTag({ content: runtimeSource });
  const observation = await observe(page);
  expect(observation.elements.map(element => element.name)).toEqual(['Billing address', 'Shipping address']);
});

test('agent can verify current form values while password values remain private', async ({ page }) => {
  await page.setContent(`<!doctype html>
    <label>Project name<input value="Before"></label>
    <label>Project notes<textarea>Initial notes</textarea></label>
    <label>Password<input type="password" value="secret-fixture-value"></label>`);
  await page.addScriptTag({ content: runtimeSource });
  await act(page, await observe(page), 'Project name', 'type', 'After');
  await act(page, await observe(page), 'Project notes', 'type', 'Saved notes');
  const observation = await observe(page);
  expect(observation.elements.find(element => element.name === 'Project name')?.value).toBe('After');
  expect(observation.elements.find(element => element.name === 'Project notes')?.value).toBe('Saved notes');
  expect(JSON.stringify(observation)).not.toContain('secret-fixture-value');
});

test('agent completes observe edit verify submit using the values the page actually accepted', async ({ page }) => {
  await page.setContent(`<!doctype html>
    <span id="project-label">Project</span><input id="project" aria-labelledby="project-label" value="Original" oninput="this.value=this.value.trim()">
    <label>Notes<textarea id="notes"></textarea></label>
    <button onclick="document.getElementById('receipt').textContent = 'Saved '+document.getElementById('project').value+': '+document.getElementById('notes').value">Save project</button>
    <p id="receipt"></p>`);
  await page.addScriptTag({ content: runtimeSource });
  await act(page, await observe(page), 'Project', 'type', '  Reviewed  ');
  const accepted = await observe(page);
  expect(accepted.elements.find(element => element.name === 'Project')?.value).toBe('Reviewed');
  await act(page, accepted, 'Notes', 'type', 'Complete');
  const readyToSubmit = await observe(page);
  expect(readyToSubmit.elements.find(element => element.name === 'Notes')?.value).toBe('Complete');
  await act(page, readyToSubmit, 'Save project', 'click');
  const committed = await page.evaluate(() => (window as unknown as {
    __NEXA_BROWSER_RUNTIME__: { observe(): { text: string } };
  }).__NEXA_BROWSER_RUNTIME__.observe());
  expect(committed.text).toContain('Saved Reviewed: Complete');
});

test('scrolling a long work queue makes its visible action reachable by agent refs', async ({ page }) => {
  await page.setContent(`<!doctype html><style>button { display:block; height:32px; }</style>
    ${Array.from({ length: 340 }, (_, index) => `<button>Earlier record ${index}</button>`).join('')}
    <button id="target">Export reviewed report</button>`);
  await page.addScriptTag({ content: runtimeSource });
  await page.locator('#target').scrollIntoViewIfNeeded();
  const observation = await observe(page);
  expect(observation.elements.some(element => element.name === 'Export reviewed report')).toBe(true);
  expect(observation.elements.length).toBeLessThanOrEqual(400);
});

test('agent recovers omitted controls with query and stable viewport pagination', async ({ page }) => {
  await page.setContent(`<!doctype html><style>button { display:block; height:30px; }</style>
    ${Array.from({ length: 340 }, (_, index) => `<button>Review record ${index}</button>`).join('')}
    <button onclick="this.textContent='Export ready'">Export reviewed report</button>`);
  await page.addScriptTag({ content: runtimeSource });
  const first = await observe(page);
  expect(first.observationCoverage).toMatchObject({ offset: 0, returned: 300, totalMatches: 341, hasMore: true, nextOffset: 300 });
  const next = await observe(page, { offset: first.observationCoverage.nextOffset! });
  expect(next.observationCoverage).toMatchObject({ offset: 300, returned: 41, hasMore: false });
  expect(new Set([...first.elements, ...next.elements].map(element => element.ref)).size).toBe(341);
  const focused = await observe(page, { query: 'EXPORT REVIEWED' });
  expect(focused.elements.map(element => element.name)).toEqual(['Export reviewed report']);
  // Native screenshot confirmation must use exactly the same options, leaving
  // the advertised refs valid and keeping the query target in the stale fence.
  await page.screenshot();
  await observe(page, { query: 'EXPORT REVIEWED' });
  await act(page, focused, 'Export reviewed report', 'click');
  expect((await observe(page, { query: 'Export ready' })).elements).toHaveLength(1);
});

test('filtered observations still reject recycled row context before dispatch', async ({ page }) => {
  await page.setContent('<!doctype html><div role="row"><span id="record">Account A</span><button onclick="window.dispatched=true">Delete account</button></div>');
  await page.addScriptTag({ content: runtimeSource });
  const before = await observe(page, { query: 'Delete account' });
  await page.locator('#record').evaluate(element => { element.textContent = 'Account B'; });
  await expect(act(page, before, 'Delete account', 'click')).rejects.toThrow(/stale observation/);
  expect(await page.evaluate(() => (window as Window & { dispatched?: boolean }).dispatched)).toBeUndefined();
});

test('standalone observation also resolves labels and suppresses private field values', async ({ page }) => {
  const coreSource = readFileSync(join(process.cwd(), '..', '..', 'crates', 'core', 'src', 'tools', 'browser_session_tool.rs'), 'utf8');
  const extractor = coreSource.match(/const INTERACTIVE_ELEMENTS_SCRIPT: &str = r#"\r?\n([\s\S]*?)\r?\n"#;/)?.[1];
  if (!extractor) throw new Error('Could not extract standalone browser observation');
  await page.setContent('<span id="label">Project</span><input aria-labelledby="label" value="Reviewed"><input type="password" value="private-password-value">');
  const elements = await page.evaluate(script => (new Function(`return (${script})`))()('input'), extractor);
  expect(elements[0]).toMatchObject({ name: 'Project', value: 'Reviewed' });
  expect(JSON.stringify(elements)).not.toContain('private-password-value');
});

test('filtered no-op checkbox preparation retains the same set for effect verification', async ({ page }) => {
  await page.setContent('<button>Other action</button><label>Already approved<input type="checkbox" checked></label>');
  await page.addScriptTag({ content: runtimeSource });
  const before = await observe(page, { query: 'Already approved' });
  const target = before.elements[0];
  const prepared = await page.evaluate(input => (window as unknown as {
    __NEXA_BROWSER_RUNTIME__: { prepareNativePointer(input: unknown): { stateMatched: boolean; verificationBaseline: { domFingerprint: string; observationOptions: ObservationOptions } } };
  }).__NEXA_BROWSER_RUNTIME__.prepareNativePointer(input), { action: 'set_checked', checked: true, targetRef: target.ref, expected: target, userEpoch: before.userEpoch, interactionFingerprint: before.interactionFingerprint });
  expect(prepared.stateMatched).toBe(true);
  expect(prepared.verificationBaseline.observationOptions).toEqual({ query: 'Already approved', offset: 0 });
  const after = await observe(page, prepared.verificationBaseline.observationOptions);
  expect(after.domFingerprint).toBe(prepared.verificationBaseline.domFingerprint);
  expect(after.elements[0].checked).toBe(true);
});

test('a queried select beyond the first control page remains verifiable after selection', async ({ page }) => {
  await page.setContent(`<style>button {display:block;height:30px}</style>${Array.from({ length: 340 }, (_, i) => `<button>Earlier ${i}</button>`).join('')}<label>Queue mode<select><option value="pending">Pending</option><option value="done">Done</option></select></label>`);
  await page.addScriptTag({ content: runtimeSource });
  expect((await observe(page)).elements.some(element => element.role === 'combobox')).toBe(false);
  const before = await observe(page, { query: 'Queue mode' });
  expect(before.elements[0].role).toBe('combobox');
  await act(page, before, before.elements[0].name, 'select', undefined, { value: 'done' });
  const after = await observe(page, { query: before.observationCoverage.query, offset: before.observationCoverage.offset });
  expect(after.elements).toHaveLength(1);
  expect(after.elements[0].ref).toBe(before.elements[0].ref);
  expect(after.elements[0].selectedValues).toEqual(['done']);
});

test('inaccessible embedded controls report the browser boundary instead of an empty-page claim', async ({ page }) => {
  await page.route('https://outer.example.test/', route => route.fulfill({ contentType: 'text/html', body: '<iframe title="Authorization provider" src="https://inner.example.test/login?state=private-frame-state#private-frame-fragment"></iframe>' }));
  await page.route('https://inner.example.test/**', route => route.fulfill({ contentType: 'text/html', body: '<button>Continue authorization</button>' }));
  await page.goto('https://outer.example.test/');
  await page.frameLocator('iframe').getByRole('button', { name: 'Continue authorization' }).waitFor({ state: 'visible' });
  await page.addScriptTag({ content: runtimeSource });
  const observation = await observe(page, { query: 'Continue authorization' });
  expect(observation.elements).toEqual([]);
  expect(observation.frameLimitations).toMatchObject({
    unavailableCount: 1,
    detailsOmitted: false,
    frames: [{ reason: 'cross_origin_or_sandboxed_frame', url: 'https://inner.example.test/login', title: 'Authorization provider', visible: true }],
  });
  expect(JSON.stringify(observation)).not.toContain('private-frame-state');
  expect(JSON.stringify(observation)).not.toContain('private-frame-fragment');
  const coreSource = readFileSync(join(process.cwd(), '..', '..', 'crates', 'core', 'src', 'tools', 'browser_session_tool.rs'), 'utf8');
  const frameScript = coreSource.match(/const FRAME_LIMITATIONS_SCRIPT: &str = r#"\r?\n([\s\S]*?)\r?\n"#;/)?.[1];
  if (!frameScript) throw new Error('Could not extract standalone frame observation');
  const standalone = await page.evaluate(script => (new Function(`return (${script})`))(), frameScript);
  expect(standalone).toEqual(observation.frameLimitations);
});
