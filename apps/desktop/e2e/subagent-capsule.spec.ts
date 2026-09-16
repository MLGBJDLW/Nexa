import { expect, test } from '@playwright/test';

test('closed subagents settle the capsule and remain settled when restored from history', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('nexa-locale', 'en'));
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/e2e/fixtures/subagent-capsule.tsx';
    const { renderSubagentCapsule } = await import(/* @vite-ignore */ path);
    renderSubagentCapsule();
  });
  const board = page.getByTestId('task-board');
  await board.getByTestId('task-board-collapsed').click();
  const rows = board.getByTestId('task-board-subtask');
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toHaveAttribute('data-status', 'running');
  await expect(board).not.toContainText('PRIVATE');
  await expect(rows.nth(0)).toContainText('Audit renderer');
  expect((await rows.nth(0).innerText()).length).toBeLessThan(100);
  await page.getByRole('button', { name: 'Queue worker input', exact: true }).click();
  await expect(rows.nth(0)).toHaveAttribute('data-status', 'running');
  await expect(board.getByTestId('plan-subagent-status')).toContainText('0/2');
  await page.getByRole('button', { name: 'Observe completed worker', exact: true }).click();
  await expect(rows.nth(0)).toHaveAttribute('data-status', 'completed');
  await expect(rows.nth(1)).toHaveAttribute('data-status', 'running');
  await expect(board.getByTestId('plan-subagent-status')).toContainText('1/2');
  await page.getByRole('button', { name: 'Close all workers', exact: true }).click();
  await expect(rows.nth(0)).toHaveAttribute('data-status', 'completed');
  await expect(rows.nth(1)).toHaveAttribute('data-status', 'cancelled');
  await expect(board.getByTestId('plan-subagent-status')).toContainText('2/2');
  await expect(board.locator('.animate-spin, .animate-pulse')).toHaveCount(0);
  await page.getByRole('button', { name: 'Reopen saved turn', exact: true }).click();
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toHaveAttribute('data-status', 'completed');
  await expect(rows.nth(1)).toHaveAttribute('data-status', 'cancelled');
  await expect(board).not.toContainText('PRIVATE');
});
