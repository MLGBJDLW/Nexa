import { expect, test, type Locator } from '@playwright/test';

async function openSettledWorker(fixture: Locator) {
  // Mode changes briefly retain the expanded outgoing trace during its exit
  // animation. Select the incoming collapsed trace, then wait for that old
  // trace to leave before addressing the nested tool card.
  await fixture.locator('[data-testid="thinking-trace-toggle"][data-trace-state="complete"][aria-expanded="false"]').click();
  const thinking = fixture.getByTestId('thinking-trace-toggle');
  await expect(thinking).toHaveCount(1);
  await expect(thinking).toHaveAttribute('aria-expanded', 'true');
  const tool = fixture.getByRole('button', { name: /spawn[_ ]subagent/i });
  await expect(tool).toBeVisible();
  if (await tool.getAttribute('aria-expanded') !== 'true') await tool.click();
  await expect(fixture.getByTestId('subagent-card')).toBeVisible();
}

test('a done nonblocking spawn follows its active parent through queue and cancellation', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('nexa-locale', 'en'));
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/e2e/fixtures/subagent-parent-lifecycle.tsx';
    const { renderSubagentParentLifecycle } = await import(/* @vite-ignore */ path);
    renderSubagentParentLifecycle();
  });
  const fixture = page.getByTestId('subagent-parent-fixture');
  await fixture.getByRole('button', { name: /spawn[_ ]subagent/i }).click();
  const card = fixture.getByTestId('subagent-card');
  await expect(card).toContainText('Queued');
  await expect(card).toHaveAttribute('aria-busy', 'true');
  await expect(card).not.toContainText('Interrupted');
  await expect(card.getByTestId('subagent-parent-controls')).toBeVisible();
  await fixture.getByRole('button', { name: 'Admit worker', exact: true }).click();
  await expect(card.getByTestId('subagent-card-trigger')).toHaveAccessibleName(/Running/);
  await fixture.getByRole('button', { name: 'Cancel worker', exact: true }).click();
  await expect(card).toContainText('Cancelling');
  await fixture.getByRole('button', { name: 'Finish parent run', exact: true }).click();
  await openSettledWorker(fixture);
  await expect(card).toContainText('Status unverified');
  await expect(card).not.toContainText('Interrupted by restart');
  await expect(card).toHaveAttribute('aria-busy', 'false');
  await expect(card.getByTestId('subagent-parent-controls')).toHaveCount(0);
  const board = fixture.getByTestId('task-board');
  await board.getByTestId('task-board-collapsed').click();
  await expect(board.getByTestId('task-board-subtask')).toHaveAttribute('data-status', 'unverified');
  await expect(board.locator('.animate-spin, .animate-pulse')).toHaveCount(0);
  await fixture.getByRole('button', { name: 'Restore saved history', exact: true }).click();
  await openSettledWorker(fixture);
  await expect(card).toContainText('Status unverified');
  await fixture.getByRole('button', { name: 'Receive terminal evidence', exact: true }).click();
  await openSettledWorker(fixture);
  await expect(card).toContainText('Complete');
  await expect(card).not.toContainText('Status unverified');
});
