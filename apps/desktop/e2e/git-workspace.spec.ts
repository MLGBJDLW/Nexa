import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale', 'en');
    const fixture = {
      snapshot: { repos: [{ sourceId: 'repo', root: 'D:/workspace', branch: 'feature/working-tree', oid: 'abcdef012345', upstream: 'origin/master', ahead: 2, behind: 1, truncated: false,
        files: [{ path: 'src/中文 file.ts', index: 'M', worktree: '.' }] }], issues: [] as unknown[], checkedSources: 1 },
      error: '', delay: 0, calls: 0, diffs: 0, diff: '+verified change', active: 0, maxActive: 0,
    };
    Object.assign(window, { gitFixture: fixture, __TAURI_INTERNALS__: {
      invoke: async (command: string, args: Record<string, unknown>) => {
        if (command === 'conversation_git_status_cmd') {
          fixture.calls += 1;
          fixture.active += 1;
          fixture.maxActive = Math.max(fixture.maxActive, fixture.active);
          const result = args.conversationId === 'git-b'
            ? { repos: [], issues: [], checkedSources: 0 } : structuredClone(fixture.snapshot);
          try {
            if (fixture.delay) await new Promise(resolve => setTimeout(resolve, fixture.delay));
            if (fixture.error) throw new Error(fixture.error);
            return result;
          } finally { fixture.active -= 1; }
        }
        if (command === 'conversation_git_diff_cmd') { fixture.diffs += 1; return fixture.diff; }
        return null;
      },
    } });
  });
});

test('shows branch, tracking, partial failure and diff without redundant refresh work', async ({ page }, testInfo) => {
  await page.goto('/e2e/fixtures/git-workspace.html');
  await expect(page.getByTestId('git-workspace-summary')).toContainText('feature/working-tree');
  await page.getByTestId('task-board-collapsed').click();
  await expect(page.getByTestId('git-workspace-details')).toContainText('↑2 ↓1');
  await page.getByRole('button', { name: 'Staged', exact: true }).click();
  await expect(page.getByTestId('git-diff-preview')).toContainText('+verified change');
  await page.evaluate(() => {
    const fixture = (window as any).gitFixture;
    fixture.snapshot.issues = [{ sourceId: 'gone', root: 'D:/missing', message: 'Folder unavailable' }];
    for (let i = 0; i < 30; i++) window.dispatchEvent(new Event('focus'));
  });
  await expect(page.getByTestId('git-workspace-error')).toHaveCount(1);
  await expect(page.getByTestId('git-workspace-details')).toContainText('Folder unavailable');
  expect(await page.evaluate(() => (window as any).gitFixture.calls)).toBe(2);
  expect(await page.evaluate(() => (window as any).gitFixture.diffs)).toBe(1);
  await page.evaluate(() => { (window as any).gitFixture.diff = '+new edit with the same Git status'; });
  await page.getByRole('button', { name: 'Refresh Git status' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).gitFixture.calls)).toBe(3);
  await expect(page.getByTestId('git-diff-preview')).toContainText('+new edit with the same Git status');
  expect(await page.evaluate(() => (window as any).gitFixture.diffs)).toBe(2);
  await page.evaluate(() => { (window as any).gitFixture.diff = '+tool edit with the same Git status'; });
  await page.getByRole('button', { name: 'Tool completed' }).click();
  await expect(page.getByTestId('git-diff-preview')).toContainText('+tool edit with the same Git status');
  await page.getByTestId('task-board-expanded').screenshot({ path: testInfo.outputPath('git-partial-status.png') });
  await page.setViewportSize({ width: 390, height: 800 });
  await expect.poll(() => page.getByTestId('task-board-expanded').evaluate(el => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
});

test('explains no repository and no linked directory instead of silently hiding detection', async ({ page }) => {
  await page.goto('/e2e/fixtures/git-workspace.html');
  await expect(page.getByTestId('git-workspace-summary')).toBeVisible();
  await page.evaluate(() => { (window as any).gitFixture.snapshot.repos = []; });
  await page.getByRole('button', { name: 'Tool completed' }).click();
  await expect(page.getByTestId('git-workspace-summary')).toHaveCount(0);
  await page.getByTestId('task-board-collapsed').click();
  await expect(page.getByTestId('git-workspace-details')).toContainText('No Git repository');
  await page.getByRole('button', { name: 'Switch conversation' }).click();
  await page.getByTestId('task-board-collapsed').click();
  await expect(page.getByTestId('git-workspace-details')).toContainText('Link a source folder');
});

test('invalidations during a scan rerun once and never leak across conversations', async ({ page }) => {
  await page.goto('/e2e/fixtures/git-workspace.html');
  await expect(page.getByTestId('git-workspace-summary')).toBeVisible();
  await page.evaluate(() => { (window as any).gitFixture.delay = 600; });
  await page.getByRole('button', { name: 'Tool completed' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).gitFixture.active)).toBe(1);
  await page.evaluate(() => { for (let i = 0; i < 30; i++) window.dispatchEvent(new Event('focus')); });
  await expect.poll(() => page.evaluate(() => (window as any).gitFixture.calls)).toBe(3);
  await page.getByRole('button', { name: 'Switch conversation' }).click();
  await page.getByTestId('task-board-collapsed').click();
  await expect(page.getByTestId('git-workspace-details')).toContainText('Link a source folder');
  await expect(page.getByTestId('git-workspace-summary')).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).gitFixture.maxActive)).toBe(1);
});

test('Git unavailable stays visible and recovers after retry', async ({ page }) => {
  await page.goto('/e2e/fixtures/git-workspace.html');
  await expect(page.getByTestId('git-workspace-summary')).toBeVisible();
  await page.evaluate(() => { (window as any).gitFixture.error = 'Git executable was not found'; });
  await page.getByRole('button', { name: 'Tool completed' }).click();
  await expect(page.getByTestId('git-workspace-error')).toBeVisible();
  await page.getByTestId('task-board-collapsed').click();
  await expect(page.getByTestId('git-workspace-details')).toContainText('Git executable was not found');
  await page.evaluate(() => { (window as any).gitFixture.error = ''; });
  await page.getByRole('button', { name: 'Refresh Git status' }).click();
  await expect(page.getByTestId('git-workspace-error')).toHaveCount(0);
  await expect(page.getByTestId('git-workspace-details')).not.toContainText('Git executable was not found');
});
