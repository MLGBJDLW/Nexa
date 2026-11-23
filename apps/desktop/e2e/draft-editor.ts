import { expect, type Locator } from '@playwright/test';

export function draftText(editor: Locator) {
  return editor.locator('.cm-line').evaluateAll(lines => lines.map(line => line.textContent).join('\n'));
}

export async function expectDraft(editor: Locator, expected: string | RegExp) {
  await expect(editor).toBeVisible();
  const text = expect.poll(() => draftText(editor));
  if (typeof expected === 'string') await text.toBe(expected);
  else await text.toMatch(expected);
}
