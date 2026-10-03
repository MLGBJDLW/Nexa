import { test, expect } from '@playwright/test';

test('OAuth uses an explicit system-browser action and keeps late sign-in replies cancelled', async ({ page }, testInfo) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale', 'en');
    let state = { connectorId: 'oauth-fixture', config: null as unknown, status: 'not_configured', authorizationEpoch: 0, expiresAt: null, scopes: [], detail: null, loginId: null as string | null };
    let finishBegin: ((value: unknown) => void) | undefined;
    const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
    Object.assign(window, { __oauthCalls: calls, __TAURI_INTERNALS__: {
      invoke: async (command: string, args: Record<string, unknown>) => {
        calls.push({ command, args });
        if (command === 'get_mcp_oauth_status_cmd') return { ...state };
        if (command === 'configure_mcp_oauth_cmd') { state = { ...state, config: args.config, status: args.config ? 'signed_out' : 'not_configured', authorizationEpoch: state.authorizationEpoch + 1 }; return { ...state }; }
        if (command === 'begin_mcp_oauth_cmd') { state = { ...state, status: 'authorizing', loginId: 'login-1' }; return new Promise((resolve) => { finishBegin = resolve; }); }
        if (command === 'disconnect_mcp_oauth_cmd') {
          state = { ...state, status: 'disconnected', loginId: null, authorizationEpoch: state.authorizationEpoch + 1 };
          setTimeout(() => finishBegin?.({ ...state, status: 'authorizing', loginId: 'old-login' }), 100);
          return { localDisconnected: true, remoteRevocation: args.revokeRemote ? 'failed' : 'not_requested' };
        }
        return null;
      }, convertFileSrc: (path: string) => path,
    } });
  });
  await page.goto('/e2e/fixtures/mcp-oauth.html');
  await page.getByText('OAuth sign-in', { exact: true }).click();
  await expect(page.getByRole('status')).toHaveText('OAuth is not configured');
  await page.getByLabel('Client ID (optional)', { exact: true }).fill('desktop-client');
  await page.getByLabel('Scopes (space separated)', { exact: true }).fill('read');
  await expect(page.getByRole('button', { name: 'Sign in', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: 'Save OAuth settings', exact: true }).click();
  await expect(page.getByRole('status')).toHaveText('Signed out');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Cancel', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.getByRole('status')).toHaveText('Disconnected');
  await expect.poll(async () => page.evaluate(() => (window as unknown as { __oauthCalls: Array<{ command: string }> }).__oauthCalls.filter((call) => call.command === 'disconnect_mcp_oauth_cmd').length)).toBe(1);
  await page.getByLabel('Client ID (optional)', { exact: true }).fill('desktop-client-updated');
  await page.getByRole('button', { name: 'Save OAuth settings', exact: true }).click();
  await expect(page.getByRole('status')).toHaveText('Signed out');
  await page.getByRole('button', { name: 'Disconnect', exact: true }).click();
  await expect(page.getByText('Disconnected locally; remote revocation failed.', { exact: true })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('mcp-oauth.png'), fullPage: true });
  const calls = await page.evaluate(() => (window as unknown as { __oauthCalls: Array<{ command: string; args: Record<string, unknown> }> }).__oauthCalls);
  expect(calls.filter((call) => call.command === 'begin_mcp_oauth_cmd')).toHaveLength(1);
  expect(calls.find((call) => call.command === 'configure_mcp_oauth_cmd')?.args.config).toEqual({ clientId: 'desktop-client', scopes: ['read'], issuer: null, resource: null, redirectPort: null });
});
