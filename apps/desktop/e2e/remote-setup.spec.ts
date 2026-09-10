import { expect, test } from '@playwright/test';
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale', 'zh-CN');
    localStorage.setItem('theme', 'light');
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, { event: string; handler: number }>();
    let next = 1; let nextListener = 1; let delay = false; let rejectStart: ((error: Error) => void) | undefined;
    const calls: { cmd: string; args: Record<string, unknown> }[] = [];
    const status = { enabled: false, preparing: false, manifest: null as unknown, devices: [] as { id: string; name: string; createdAt: string }[], warning: null, certificatePath: null, tunnelRunning: false, connectedDeviceIds: [] as string[], pairingDeviceId: null as string | null };
    const changed = () => { for (const [id, listener] of listeners) if (listener.event === 'remote:status-changed') callbacks.get(listener.handler)?.({ event: listener.event, id, payload: null }); };
    const invoke = async (cmd: string, args: Record<string, unknown> = {}) => {
      if (cmd.startsWith('remote_') || cmd.includes('_remote_')) calls.push({ cmd, args });
      switch (cmd) {
        case 'plugin:event|listen': { const id = nextListener++; listeners.set(id, { event: String(args.event), handler: Number(args.handler) }); return id; }
        case 'plugin:event|unlisten': listeners.delete(Number(args.eventId)); return null;
        case 'get_wizard_state_cmd': return { completed: true, completedAt: new Date().toISOString() };
        case 'remote_status_cmd': return structuredClone(status);
        case 'start_remote_cmd': status.preparing = true; changed(); if (delay) await new Promise<void>((_, reject) => { rejectStart = reject; }); status.preparing = false; status.enabled = true; status.tunnelRunning = true; status.manifest = { serverId: 'desktop', endpoints: [{ kind: 'lan', url: 'https://192.168.1.8:8791' }, { kind: 'tunnel', url: 'https://sample.trycloudflare.com' }] }; changed(); return structuredClone(status);
        case 'stop_remote_cmd': status.enabled = false; status.preparing = false; status.manifest = null; status.devices = []; status.connectedDeviceIds = []; status.pairingDeviceId = null; rejectStart?.(new Error('Remote setup was cancelled')); changed(); return null;
        case 'remote_pairing_cmd': status.pairingDeviceId = null; changed(); return { pairing: { code: '483921', expiresAt: new Date(Date.now() + 180000).toISOString() }, url: 'https://sample.trycloudflare.com/#pair=483921&server=desktop', qrSvg: '<svg xmlns="http://www.w3.org/2000/svg" width="280" height="280"><rect width="280" height="280" fill="white"/><path fill="black" d="M20 20h80v80H20zm160 0h80v80h-80zM20 180h80v80H20zm105-50h35v35h-35zm55 50h25v80h-25zm40 0h40v25h-40zm0 40h40v40h-40z"/></svg>' };
        case 'revoke_remote_device_cmd': status.devices = []; status.connectedDeviceIds = []; status.pairingDeviceId = null; changed(); return null;
        case 'list_agent_configs_cmd': case 'list_sources': case 'list_conversations_cmd': case 'list_personas_cmd': case 'list_projects_cmd': case 'list_skills_cmd': return [];
        default: return null;
      }
    };
    Object.assign(window, { __remoteSetupCalls: calls, __delayRemoteSetup: () => { delay = true; }, __pairPhone: () => { status.devices = [{ id: 'phone-1', name: '我的 iPhone', createdAt: new Date().toISOString() }]; status.connectedDeviceIds = ['phone-1']; status.pairingDeviceId = 'phone-1'; changed(); } });
    Object.assign(window, { __TAURI_INTERNALS__: { invoke, metadata: { currentWindow: { label: 'main' } }, transformCallback: (callback: (event: unknown) => void) => { const id = next++; callbacks.set(id, callback); return id; }, unregisterCallback: (id: number) => callbacks.delete(id), convertFileSrc: (path: string) => path }, __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: (_: string, id: number) => listeners.delete(id) } });
  });
});
test('phone access stays above Settings and guides enable, scan, connected, and revoke', async ({ page }) => {
  await page.goto('/remote');
  const entry = page.getByTestId('remote-sidebar-link');
  const settings = page.getByRole('link', { name: '设置', exact: true });
  await expect(entry).toHaveAttribute('data-remote-status', 'disabled');
  expect(await entry.evaluate(node => node.closest('[data-theme-density-part="rail-footer"]') !== null)).toBe(true);
  const entryBounds = await entry.boundingBox(); const settingsBounds = await settings.boundingBox();
  expect(entryBounds!.y + entryBounds!.height).toBeLessThanOrEqual(settingsBounds!.y + 1);
  await expect(page.getByTestId('remote-advanced')).not.toHaveAttribute('open');
  expect(await page.evaluate(() => (window as any).__remoteSetupCalls.filter((call: any) => call.cmd === 'start_remote_cmd').length)).toBe(0);
  await page.screenshot({ path: '.artifacts/remote-desktop-disabled.png', fullPage: true });
  await page.getByRole('button', { name: '启用并显示二维码', exact: true }).click();
  await expect(page.getByAltText('Nexa 一次性配对二维码')).toBeVisible();
  await expect(entry).toHaveAttribute('data-remote-status', 'ready');
  expect(await page.evaluate(() => (window as any).__remoteSetupCalls.find((call: any) => call.cmd === 'start_remote_cmd').args.options)).toEqual({ lan: true, quickTunnel: true, publicUrl: null });
  await page.screenshot({ path: '.artifacts/remote-desktop-pair.png', fullPage: true });
  await page.evaluate(() => (window as any).__pairPhone());
  await expect(page.getByTestId('remote-pair-card')).toContainText('设备已配对');
  await expect(page.getByAltText('Nexa 一次性配对二维码')).toHaveCount(0);
  await expect(entry).toHaveAttribute('data-remote-status', 'connected');
  await page.screenshot({ path: '.artifacts/remote-desktop-connected.png', fullPage: true });
  await page.getByRole('button', { name: '添加另一台设备', exact: true }).click();
  await expect(page.getByAltText('Nexa 一次性配对二维码')).toBeVisible();
  await page.getByRole('button', { name: '撤销连接', exact: true }).click();
  await expect(entry).toHaveAttribute('data-remote-status', 'ready');
});
test('connection preparation can be cancelled and does not show a stale QR code', async ({ page }) => {
  await page.goto('/remote');
  await page.evaluate(() => (window as any).__delayRemoteSetup());
  await page.getByRole('button', { name: '启用并显示二维码', exact: true }).click();
  await page.getByRole('button', { name: '取消准备', exact: true }).click();
  await expect(page.getByRole('button', { name: '启用并显示二维码', exact: true })).toBeEnabled();
  await expect(page.getByAltText('Nexa 一次性配对二维码')).toHaveCount(0);
  await expect(page.getByTestId('remote-sidebar-link')).toHaveAttribute('data-remote-status', 'disabled');
});
