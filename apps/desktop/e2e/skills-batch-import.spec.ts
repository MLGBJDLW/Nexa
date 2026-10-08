import { expect, test } from '@playwright/test';

test('one picker selection previews and installs every selected skill source', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale', 'en');
    const sources = ['C:/fixtures/alpha.skill', 'C:/fixtures/beta.zip'];
    const skills = ['alpha', 'beta'].map((name, index) => ({
      name, description: `${name} skill`, skillDir: sources[index], contentDigest: `digest-${name}`,
      skillFile: `${sources[index]}!/${name}/SKILL.md`, warnings: [], resources: [],
    }));
    Object.assign(window, {
      __skillImportCalls: [] as Array<{ command: string; args: unknown }>,
      __TAURI_INTERNALS__: { invoke: async (command: string, args: Record<string, unknown>) => {
        (window as any).__skillImportCalls.push({ command, args });
        if (command === 'plugin:dialog|open') return sources;
        if (command === 'inspect_skill_install_sources_cmd') return skills;
        if (command === 'install_skills_from_sources_cmd') return skills;
        if (command === 'inspect_skill_install_source_cmd') return [skills[0]];
        return null;
      } },
    });
  });
  await page.goto('/e2e/fixtures/skill-installer.html');
  await page.getByRole('button', { name: /Install/ }).click();
  const dialog = page.getByTestId('skill-installer');
  await dialog.getByRole('button', { name: /\.skill, \.zip, SKILL\.md/ }).click();
  await expect(dialog.getByTestId('skill-install-preview')).toContainText('alpha');
  await expect(dialog.getByTestId('skill-install-preview')).toContainText('beta');
  await expect.poll(() => page.evaluate(() => (window as any).__skillImportCalls.find((call: any) => call.command === 'plugin:dialog|open')?.args.options.multiple)).toBe(true);
  await dialog.getByRole('button', { name: /Install.*2|2.*skills/i }).click();
  await expect(page.getByTestId('installation-refresh-count')).toHaveText('1');
  const installed = await page.evaluate(() => (window as any).__skillImportCalls.find((call: any) => call.command === 'install_skills_from_sources_cmd'));
  expect(installed.args.sources).toEqual(['C:/fixtures/alpha.skill', 'C:/fixtures/beta.zip']);
});

test('a collection preview installs only selected skills and scopes risk acknowledgement to that selection', async ({ page }, testInfo) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale', 'en');
    const skills = ['safe-alpha', 'safe-beta', 'needs-review'].map(name => ({
      name, description: `Reusable ${name} workflow`, skillDir: `collection.zip!/${name}`,
      skillFile: `collection.zip!/${name}/SKILL.md`, contentDigest: `digest-${name}`, resources: [],
      warnings: name === 'needs-review' ? [{ severity: 'block', code: 'fixture.warning', message: 'Fixture requires review.' }] : [],
    }));
    Object.assign(window, { __skillImportCalls: [], __TAURI_INTERNALS__: { invoke: async (command: string, args: Record<string, unknown>) => {
      (window as any).__skillImportCalls.push({ command, args });
      if (command === 'plugin:dialog|open') return 'C:/fixtures/collection.zip';
      if (command === 'inspect_skill_install_sources_cmd') return skills;
      if (command === 'install_skills_from_sources_cmd') return [skills[0]];
      return null;
    } } });
  });
  await page.goto('/e2e/fixtures/skill-installer.html');
  await page.getByRole('button', { name: 'Install', exact: true }).click();
  const dialog = page.getByTestId('skill-installer');
  await dialog.getByRole('button', { name: /\.skill, \.zip, SKILL\.md/ }).click();
  await expect(dialog.getByRole('button', { name: 'Install 3', exact: true })).toBeDisabled();
  await dialog.getByRole('checkbox', { name: 'needs-review', exact: true }).uncheck();
  await dialog.getByRole('checkbox', { name: 'safe-beta', exact: true }).uncheck();
  await expect(dialog.getByRole('button', { name: 'Install 1', exact: true })).toBeEnabled();
  await expect(dialog.getByText('Fixture requires review.', { exact: false })).toHaveCount(0);
  await page.setViewportSize({ width: 760, height: 800 });
  await dialog.screenshot({ path: testInfo.outputPath('selected-skill-collection.png') });
  await dialog.getByRole('button', { name: 'Install 1', exact: true }).click();
  await expect(page.getByTestId('installation-refresh-count')).toHaveText('1');
  const installed = await page.evaluate(() => (window as any).__skillImportCalls.find((call: any) => call.command === 'install_skills_from_sources_cmd'));
  expect(installed.args.selection).toEqual([{ skillFile: 'collection.zip!/safe-alpha/SKILL.md', contentDigest: 'digest-safe-alpha' }]);
  expect(installed.args.acceptBlockedWarnings).toBe(false);
});
