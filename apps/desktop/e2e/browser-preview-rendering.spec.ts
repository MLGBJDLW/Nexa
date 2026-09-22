import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from '@playwright/test';

const serverSource = readFileSync(join(process.cwd(), 'src-tauri/src/browser/local_html.rs'), 'utf8');
const previewPolicy = serverSource.match(/header::CONTENT_SECURITY_POLICY, "([^"]+)"/)?.[1];
if (!previewPolicy) throw new Error('Could not extract the local HTML preview response policy');

test('local HTML preview supports module components and blob workers under its real response policy', async ({ page }, testInfo) => {
  await page.route('https://nexa-preview.test/**', route => {
    if (route.request().url().endsWith('/component.mjs')) {
      return route.fulfill({ contentType: 'text/javascript', body: `
        customElements.define('render-card', class extends HTMLElement {
          connectedCallback() {
            const root = this.attachShadow({ mode: 'open' });
            root.innerHTML = '<style>:host{display:grid;grid-template-columns:1fr 1fr;gap:16px;padding:24px;background:#eff6ff;font:20px sans-serif}svg{width:100%;height:120px}</style><h1>Component ready</h1><svg viewBox="0 0 200 120"><rect width="200" height="120" rx="20" fill="#0d9488"/><text x="30" y="70" fill="white">SVG rendered</text></svg>';
          }
        });
      ` });
    }
    return route.fulfill({
      contentType: 'text/html; charset=utf-8',
      headers: { 'Content-Security-Policy': previewPolicy },
      body: '<!doctype html><title>Rendering fixture</title><render-card></render-card><output id="worker-status">Starting renderer</output><script type="module" src="/component.mjs"></script>',
    });
  });
  await page.goto('https://nexa-preview.test/index.html');
  await expect(page.getByRole('heading', { name: 'Component ready' })).toBeVisible();
  const result = await page.evaluate(async () => {
    const url = URL.createObjectURL(new Blob(['onmessage = ({data}) => postMessage(data * 2)'], { type: 'text/javascript' }));
    let worker: Worker | undefined;
    try {
      return await new Promise<number | string>(resolve => {
        const timeout = setTimeout(() => resolve('worker timed out'), 3000);
        worker = new Worker(url);
        worker.onmessage = event => {
          clearTimeout(timeout);
          document.getElementById('worker-status')!.textContent = `Worker rendered ${event.data}`;
          resolve(event.data);
        };
        worker.onerror = event => { clearTimeout(timeout); resolve(event.message || 'worker rejected by preview policy'); };
        worker.postMessage(21);
      });
    } catch (error) {
      return String(error);
    } finally {
      worker?.terminate();
      URL.revokeObjectURL(url);
    }
  });
  expect(result).toBe(42);
  await expect(page.locator('#worker-status')).toHaveText('Worker rendered 42');
  await page.screenshot({ path: testInfo.outputPath('preview-components.png') });
});
