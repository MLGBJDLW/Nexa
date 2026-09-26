import { expect, test } from '@playwright/test';
import presets from '../../../shared/embedding-provider-presets.json' with { type: 'json' };

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale', 'en');
    Object.assign(window, { embeddingReady: false, __TAURI_INTERNALS__: { invoke: async (command: string) => command === 'get_embedding_index_status_cmd'
      ? { totalChunks: 20, indexedChunks: (window as any).embeddingReady ? 20 : 0, legacyChunks: 20, needsRebuild: !(window as any).embeddingReady }
      : null } });
  });
  await page.goto('/e2e/fixtures/embedding-settings.html');
  await page.getByRole('button', { name: /Embedding Configuration/ }).click();
});

test('every verified provider chooses a valid default model and matching vector dimensions', async ({ page }) => {
  for (const preset of presets.filter(preset => preset.models.length)) {
    await page.getByRole('combobox', { name: 'Provider', exact: true }).click();
    await page.getByRole('option', { name: preset.name, exact: true }).click();
    const expected = preset.models.find(model => model.recommended)!;
    const config = JSON.parse(await page.getByTestId('embedding-config').textContent() ?? '{}');
    expect(config.apiBaseUrl).toBe(preset.baseUrl);
    expect(config.apiModel).toBe(expected.id);
    expect(config.vectorDimensions).toBe(expected.dimensions);
    expect(config.apiKey).toBe('');
  }
});

test('Qwen workspace endpoints retain model choices, supported dimensions and manual IDs', async ({ page }, testInfo) => {
  await page.getByRole('combobox', { name: 'Provider', exact: true }).click();
  await page.getByRole('option', { name: 'Alibaba Model Studio Embeddings (Beijing)', exact: true }).click();
  await page.getByRole('textbox', { name: 'Base URL', exact: true }).fill('https://my-workspace.cn-beijing.maas.aliyuncs.com/compatible-mode/v1');
  await expect(page.getByRole('combobox', { name: 'Provider', exact: true })).toContainText('Alibaba');
  await page.getByRole('combobox', { name: 'Vector dimensions', exact: true }).click();
  await expect(page.getByRole('option', { name: '2048', exact: true })).toBeVisible();
  await page.getByRole('option', { name: '512', exact: true }).click();
  await expect(page.getByTestId('embedding-config')).toContainText('"vectorDimensions":512');
  await page.getByRole('checkbox', { name: 'Enter a custom model ID' }).check();
  await page.getByRole('textbox', { name: 'Model', exact: true }).fill('my-deployed-embedding');
  await expect(page.getByTestId('embedding-config')).toContainText('my-deployed-embedding');
  await page.screenshot({ path: testInfo.outputPath('embedding-qwen-workspace.png'), fullPage: true });
});

test('keyless Ollama can test locally, while remote endpoints still require credentials', async ({ page }) => {
  await expect(page.getByRole('button', { name: 'Test Connection', exact: true })).toBeDisabled();
  await page.getByRole('combobox', { name: 'Provider', exact: true }).click();
  await page.getByRole('option', { name: 'Ollama (local)', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Test Connection', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Test Connection', exact: true }).click();
  await expect(page.getByTestId('embedding-tested')).toContainText('qwen3-embedding:0.6b');
  await page.getByRole('textbox', { name: 'Base URL', exact: true }).fill('https://host.example/v1');
  await expect(page.getByRole('button', { name: 'Test Connection', exact: true })).toBeDisabled();
});

test('legacy indexes show an actionable rebuild state and refresh coverage after rebuilding', async ({ page }) => {
  await expect(page.getByTestId('embedding-index-status')).toContainText('0/20');
  await expect(page.getByTestId('embedding-index-status')).toContainText('Rebuild embeddings');
  await page.getByRole('button', { name: 'Re-embed All', exact: true }).click();
  await expect(page.getByTestId('embedding-index-status')).toContainText('20/20');
  await expect(page.getByTestId('embedding-index-status')).not.toContainText('Rebuild embeddings');
});
