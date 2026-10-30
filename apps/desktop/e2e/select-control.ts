import { expect, type Locator } from '@playwright/test';

/** Use the same keyboard/pointer boundary as the user, including popup dismissal. */
export async function selectControl(trigger: Locator, value: string) {
  if (await trigger.evaluate(element => element.tagName === 'SELECT')) {
    await trigger.selectOption(value);
    return;
  }
  await trigger.click();
  await trigger.page().locator(`[role="option"][data-value=${JSON.stringify(value)}]`).click();
  await expect(trigger).toHaveAttribute('aria-expanded', 'false');
  await expect(trigger.page().locator('[role="option"]:visible')).toHaveCount(0);
  await trigger.page().evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
}
