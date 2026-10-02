import { expect, test, type Locator } from './timeline-test';

test('user Markdown folds by rendered height, preserves source, and does not load remote images', async ({ page }) => {
  const imageRequests: string[] = [];
  page.on('request', request => { if (request.url().includes('example.invalid')) imageRequests.push(request.url()); });
  await page.goto('/chat/conv-slash?longMessage=1');
  const body = page.getByTestId('chat-user-message-body');
  const expand = body.getByRole('button', { name: 'Show more' });
  await expect(expand).toHaveAttribute('aria-expanded', 'false');
  await expect(body.locator('h1')).toHaveText('A long request');
  await expect(body.locator('strong')).toHaveText('Keep this text');
  expect(await body.getByTestId('chat-user-message-text').evaluate(el => el.parentElement!.clientHeight)).toBeLessThanOrEqual(121);
  await expand.click();
  await expect(body.getByRole('button', { name: 'Show less' })).toHaveAttribute('aria-expanded', 'true');
  await body.getByRole('button', { name: 'View source' }).click();
  await expect(body.getByTestId('chat-user-message-text')).toContainText('**Keep this text**');
  await expect(body.getByTestId('chat-user-message-text')).toContainText('<script>');
  expect(imageRequests).toEqual([]);
  expect(await page.evaluate(() => (window as unknown as { __unsafe?: boolean }).__unsafe)).toBeUndefined();
  await page.screenshot({ path: '../../.artifacts/chat-command-center/message-source.png' });
});

test('draft preview and local slash commands keep original text and never send a command to the model', async ({ page }) => {
  await page.goto('/chat/conv-slash');
  const input = page.getByTestId('chat-input-textarea');
  const source = '# Draft\n\n**中文输入**\n\n- keep raw text';
  await input.fill(source);
  await page.getByTestId('chat-preview-toggle').click();
  await expect(input).toBeHidden();
  await expect(page.getByTestId('chat-draft-preview').locator('h1')).toHaveText('Draft');
  await page.getByTestId('chat-draft-preview').getByRole('button', { name: 'Edit draft' }).click();
  await expect(input).toHaveValue(source);
  await expect(input).toBeFocused();
  await input.fill('/model');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('agent-model-picker-menu')).toBeVisible();
  await expect(input).toHaveValue('');
  await page.keyboard.press('Escape');
  await input.fill('/options');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('chat-more-options')).toHaveAttribute('aria-expanded', 'true');
  expect(await page.evaluate(() => (window as unknown as { __slashAgentChatCalls__: unknown[] }).__slashAgentChatCalls__.length)).toBe(0);
  await input.fill(source);
  await page.getByTestId('chat-send').click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __slashAgentChatCalls__: Array<{ message: string }> }).__slashAgentChatCalls__[0]?.message)).toBe(source);
});

test('keyboard preview commands transfer focus and Escape returns to the original draft', async ({ page }) => {
  await page.goto('/chat/conv-slash');
  const input = page.getByTestId('chat-input-textarea');
  const preview = page.getByTestId('chat-draft-preview');
  await input.fill('/preview');
  await page.keyboard.press('Enter');
  await expect(preview).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(input).toBeFocused();
  await expect(input).toHaveValue('');
  await input.fill('# Preserve this draft');
  await page.keyboard.press('Control+Shift+P');
  const dialog = page.getByRole('dialog', { name: /command palette/i });
  await dialog.getByRole('combobox').fill('Preview draft');
  await dialog.getByRole('option', { name: 'Preview draft', exact: true }).click();
  await expect(preview).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(input).toBeFocused();
  await expect(input).toHaveValue('# Preserve this draft');
});

for (const message of ['Please inspect the /model endpoint and explain its response.', 'The /preview route is returning 404.']) {
  test(`literal command path remains ordinary message text: ${message}`, async ({ page }) => {
    await page.goto('/chat/conv-slash');
    await page.getByTestId('chat-input-textarea').fill(message);
    await page.getByTestId('chat-send').click();
    await expect.poll(() => page.evaluate(() => (window as unknown as { __slashAgentChatCalls__: Array<{ message: string }> }).__slashAgentChatCalls__[0]?.message)).toBe(message);
  });
}

test('command palette runs chat actions, searches conversations, restores focus, and supports a persisted custom chord', async ({ page }) => {
  await page.goto('/chat/conv-slash');
  const input = page.getByTestId('chat-input-textarea');
  await input.fill('Keep my draft');
  await page.keyboard.press('Control+Shift+P');
  const dialog = page.getByRole('dialog', { name: /command palette/i });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('combobox').fill('model');
  await dialog.getByRole('option').filter({ hasText: /default model/i }).click();
  await expect(page.getByTestId('agent-model-picker-menu')).toBeVisible();
  await page.keyboard.press('Escape');
  await input.focus();
  await page.keyboard.press('Control+k');
  await dialog.getByRole('combobox').fill('Slash commands');
  await expect(dialog.getByRole('option').filter({ hasText: 'Slash commands' })).toHaveCount(2);
  await page.keyboard.press('Escape');
  await expect(input).toBeFocused();
  await expect(input).toHaveValue('Keep my draft');
  await page.goto('/settings');
  const shortcut = page.getByTestId('palette-shortcut-input');
  await shortcut.focus();
  await page.keyboard.press('Control+Alt+O');
  await expect(shortcut).toHaveValue('Ctrl+Alt+O');
  await page.reload();
  await page.getByTestId('palette-shortcut-input').waitFor();
  await page.keyboard.press('Control+Alt+O');
  await expect(dialog).toBeVisible();
  await dialog.getByRole('combobox').fill('');
  await page.screenshot({ path: '../../.artifacts/chat-command-center/palette.png' });
});

test('IME confirmation cannot submit a draft and the compact composer fits narrow screens', async ({ page }) => {
  await page.setViewportSize({ width: 480, height: 800 });
  await page.goto('/chat/conv-slash?locale=zh-CN');
  const input = page.getByTestId('chat-input-textarea');
  await input.fill('正在输入');
  await input.dispatchEvent('keydown', { key: 'Enter', code: 'Enter', isComposing: true });
  expect(await page.evaluate(() => (window as unknown as { __slashAgentChatCalls__: unknown[] }).__slashAgentChatCalls__.length)).toBe(0);
  await expect(page.getByTestId('chat-advanced-options')).toBeHidden();
  const toolbar = page.getByTestId('chat-input-toolbar');
  expect(await toolbar.evaluate(el => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
  await page.getByTestId('chat-more-options').click();
  await expect(page.getByTestId('chat-quality-profile')).toBeVisible();
  await page.getByTestId('chat-more-options').click();
  await page.screenshot({ path: '../../.artifacts/chat-command-center/composer-narrow.png' });
});

async function selectNexaOption(trigger: Locator, value: string) {
  await trigger.click();
  await trigger.page().locator(`[role="option"][data-value=${JSON.stringify(value)}]`).click();
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const testLocale = new URLSearchParams(window.location.search).get('locale') ?? 'en';
    localStorage.setItem('nexa-locale', testLocale);

    const nowIso = new Date().toISOString();
    const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;
    const agentChatCalls: Array<Record<string, unknown>> = [];

    const conversation = {
      id: 'conv-slash',
      title: 'Slash commands',
      provider: 'open_ai',
      model: 'gpt-4.1',
      systemPrompt: '',
      collectionContext: null,
      projectId: null,
      personaId: null,
      createdAt: nowIso,
      updatedAt: nowIso,
    };

    const defaultAgentConfig = {
      id: 'cfg-slash',
      name: 'Slash Config',
      provider: 'open_ai',
      apiKey: '',
      baseUrl: null,
      model: 'gpt-4.1',
      temperature: 0.3,
      maxTokens: 4096,
      contextWindow: 1047576,
      isDefault: true,
      reasoningEnabled: null,
      thinkingBudget: null,
      reasoningEffort: null,
      maxIterations: null,
      summarizationModel: null,
      summarizationProvider: null,
      subagentAllowedTools: null,
      createdAt: nowIso,
      updatedAt: nowIso,
    };

    const frontendSkill = {
      id: 'builtin-frontend-design',
      name: 'frontend-design',
      description: 'Create distinctive production-grade frontend interfaces.',
      content: 'Use this skill when building web UI.',
      enabled: true,
      createdAt: nowIso,
      updatedAt: nowIso,
      builtin: true,
      interface: {
        displayName: 'Frontend Design',
        shortDescription: 'Design and implement refined UI.',
        defaultPrompt: 'Use frontend-design for this UI task.\n\nTask:\n{{input}}',
      },
      dependencies: { tools: [] },
      policy: { allowImplicitInvocation: true },
      sourcePath: null,
      resources: [],
    };
    const extraSkills = Array.from({ length: 20 }, (_, index) => ({
      ...frontendSkill,
      id: `builtin-extra-${index}`,
      name: `z-extra-${String(index).padStart(2, '0')}`,
      interface: {
        ...frontendSkill.interface,
        displayName: `Z Extra ${index}`,
      },
    }));

    const callbackMap = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, { event: string; handlerId: number }>();
    let callbackSeq = 1;
    let listenerSeq = 1;

    const invoke = async (cmd: string, args: Record<string, unknown> = {}) => {
      if (cmd === 'agent_chat_cmd') args = (args.request as Record<string, unknown>) ?? {};
      switch (cmd) {
        case 'plugin:event|listen': {
          const listenerId = listenerSeq++;
          listeners.set(listenerId, {
            event: String(args.event ?? ''),
            handlerId: Number(args.handler ?? 0),
          });
          return listenerId;
        }
        case 'plugin:event|unlisten':
          listeners.delete(Number(args.eventId ?? 0));
          return null;
        case 'agent_chat_cmd':
          agentChatCalls.push(clone(args));
          return null;
        case 'list_workflow_templates_cmd':
          return [];
        case 'list_builtin_skills_cmd':
          return [clone(frontendSkill), ...clone(extraSkills)];
        case 'list_skills_cmd':
          return [];
        case 'list_agent_configs_cmd':
          return [clone(defaultAgentConfig)];
        case 'get_model_context_window':
          return 1047576;
        case 'get_wizard_state_cmd':
          return { completed: true, language: 'en', aiProvider: 'open_ai', sourceAdded: true };
        case 'list_conversations_cmd':
          return [clone(conversation)];
        case 'get_conversation_cmd': {
          const content = '# A long request\n\n**Keep this text**\n\n' + Array.from({ length: 18 }, (_, index) => '- Task ' + index + ': 这是原始需求。').join('\n') + '\n\n![remote](https://example.invalid/track.png)\n<script>window.__unsafe = true</script>';
          return [clone(conversation), new URLSearchParams(location.search).has('longMessage') ? [{ id: 'long-user', conversationId: conversation.id, role: 'user', content, toolCallId: null, toolCalls: [], artifacts: null, thinking: null, sortOrder: 0, tokenCount: 0, createdAt: nowIso }] : []];
        }
        case 'get_conversation_turns_cmd':
        case 'get_agent_task_runs_cmd':
        case 'list_sources':
        case 'get_conversation_sources_cmd':
        case 'list_checkpoints_cmd':
        case 'list_mcp_servers_cmd':
        case 'list_projects_cmd':
        case 'list_personas_cmd':
          return [];
        case 'get_index_stats':
          return { totalDocuments: 0, totalChunks: 0, ftsRows: 0 };
        case 'get_privacy_config':
          return { enabled: false, excludePatterns: [], redactPatterns: [] };
        case 'get_embedder_config_cmd':
          return {
            provider: 'tfidf',
            apiKey: '',
            apiBaseUrl: '',
            apiModel: '',
            localModel: '',
            modelPath: '',
            vectorDimensions: 384,
          };
        case 'get_ocr_config_cmd':
          return {
            enabled: false,
            minConfidence: 0.5,
            llmFallback: false,
            detectionLimit: 2048,
            useCls: false,
          };
        case 'check_ocr_models_cmd':
          return false;
        default:
          return null;
      }
    };

    (window as unknown as { __slashAgentChatCalls__: Array<Record<string, unknown>> }).__slashAgentChatCalls__ = agentChatCalls;
    (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
      invoke,
      transformCallback: (callback: (event: unknown) => void) => {
        const id = callbackSeq++;
        callbackMap.set(id, callback);
        return id;
      },
      unregisterCallback: (id: number) => {
        callbackMap.delete(id);
      },
      convertFileSrc: (filePath: string) => filePath,
    };
    (window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__: unknown }).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: (_event: string, eventId: number) => {
        listeners.delete(eventId);
      },
    };
  });
});

test('slash command menu can pin a skill for the next send', async ({ page }) => {
  await page.goto('/chat/conv-slash');

  const textarea = page.getByTestId('chat-input-textarea');
  await textarea.fill('/front');

  const menu = page.getByTestId('slash-command-menu');
  await expect(menu).toBeVisible();
  await expect(menu).toContainText('/frontend-design');

  await page.keyboard.press('Enter');
  await expect(textarea).toHaveValue('');
  const capsule = page.getByTestId('active-slash-command');
  await expect(capsule).toBeVisible();
  await expect(capsule).toContainText('/frontend-design');

  await textarea.fill('build a dense dashboard');
  await page.getByTestId('chat-send').click();

  await expect.poll(
    () => page.evaluate(() =>
      (window as unknown as { __slashAgentChatCalls__: Array<Record<string, unknown>> })
        .__slashAgentChatCalls__[0]?.skillIds,
    ),
  ).toEqual(['builtin-frontend-design']);
  await expect.poll(
    () => page.evaluate(() =>
      String(((window as unknown as { __slashAgentChatCalls__: Array<Record<string, unknown>> })
        .__slashAgentChatCalls__[0]?.userArtifacts as Record<string, unknown> | undefined)
        ?.llmContextContent ?? ''),
    ),
  ).toContain('Use frontend-design for this UI task.');
  await expect.poll(
    () => page.evaluate(() =>
      String(((window as unknown as { __slashAgentChatCalls__: Array<Record<string, unknown>> })
        .__slashAgentChatCalls__[0]?.userArtifacts as Record<string, unknown> | undefined)
        ?.llmContextContent ?? ''),
    ),
  ).toContain('build a dense dashboard');
  await expect.poll(
    () => page.evaluate(() =>
      String((window as unknown as { __slashAgentChatCalls__: Array<Record<string, unknown>> })
        .__slashAgentChatCalls__[0]?.message ?? ''),
    ),
  ).toBe('build a dense dashboard');
});

test('live turn timing appears after three seconds without a global elapsed state', async ({ page }) => {
  await page.goto('/chat/conv-slash');
  await page.getByTestId('chat-input-textarea').fill('measure this turn');
  await page.getByTestId('chat-send').click();

  const elapsed = page.getByTestId('chat-turn-elapsed');
  await expect(elapsed).toBeVisible({ timeout: 6000 });
  await expect(elapsed).toContainText(/Thinking · \d+:\d{2}/);
  const duration = (await elapsed.innerText()).match(/(\d+):(\d{2})/)!;
  expect(Number(duration[1]) * 60 + Number(duration[2])).toBeGreaterThanOrEqual(3);

  await page.getByTestId('chat-context-trigger').hover();
  await expect(page.getByTestId('chat-turn-timing-metrics')).toContainText('First event');
  await expect(page.getByTestId('chat-turn-timing-metrics')).toContainText('First visible output');
  await expect(page.getByTestId('chat-turn-timing-metrics')).toContainText('Total time');
});

test('an activated slash command can be cancelled without editing the prompt', async ({ page }) => {
  await page.goto('/chat/conv-slash');

  const textarea = page.getByTestId('chat-input-textarea');
  await textarea.fill('/front');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('active-slash-command')).toBeVisible();

  await page.getByTestId('remove-active-slash-command').click();

  await expect(page.getByTestId('active-slash-command')).toHaveCount(0);
  await expect(textarea).toHaveValue('');
  await expect(textarea).toBeFocused();
});

test('slash command tabs filter the second-level option list', async ({ page }) => {
  await page.goto('/chat/conv-slash');

  const textarea = page.getByTestId('chat-input-textarea');
  await textarea.fill('/');

  const menu = page.getByTestId('slash-command-menu');
  await expect(menu).toBeVisible();
  await expect(page.getByTestId('slash-command-tab-all')).toHaveAttribute('aria-selected', 'true');

  await page.getByTestId('slash-command-tab-skill').click();
  await expect(page.getByTestId('slash-command-tab-skill')).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByTestId('slash-command-option-frontend-design')).toBeVisible();
  await expect(page.getByTestId('slash-command-option-plan')).toHaveCount(0);

  await page.getByTestId('slash-command-tab-command').click();
  await expect(page.getByTestId('slash-command-option-plan')).toHaveCount(1);
  await expect(page.getByTestId('slash-command-option-frontend-design')).toHaveCount(0);
});

test('slash command menu uses the shared collision-aware overlay portal', async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 520 });
  await page.goto('/chat/conv-slash');
  await page.getByTestId('chat-input-textarea').fill('/');
  await page.waitForTimeout(180);

  const bounds = await page.getByTestId('slash-command-menu').evaluate((menu) => {
    const rect = menu.getBoundingClientRect();
    const styles = getComputedStyle(menu);
    return {
      inOverlayRoot: Boolean(menu.closest('[data-nexa-overlay-root="true"]')),
      backdropFilter: styles.backdropFilter,
      animationName: styles.animationName,
      left: rect.left,
      right: rect.right,
      top: rect.top,
      bottom: rect.bottom,
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
    };
  });

  expect(bounds.inOverlayRoot).toBe(true);
  expect(bounds.backdropFilter).toMatch(/^(none|blur\(0px\))$/);
  expect(bounds.animationName).toContain('nexa-command-overlay');
  expect(bounds.left).toBeGreaterThanOrEqual(0);
  expect(bounds.top).toBeGreaterThanOrEqual(0);
  expect(bounds.right).toBeLessThanOrEqual(bounds.viewportWidth);
  expect(bounds.bottom).toBeLessThanOrEqual(bounds.viewportHeight);
});

test('slash command menu keeps all matched commands reachable in its scroll area', async ({ page }) => {
  await page.goto('/chat/conv-slash');
  await page.getByTestId('chat-input-textarea').fill('/');

  const list = page.getByTestId('slash-command-list');
  expect(await list.getByRole('option').count()).toBeGreaterThan(16);
  expect(await list.getByRole('option').count()).toBeLessThanOrEqual(64);
  await expect(page.getByTestId('slash-command-option-nexus')).toHaveCount(1);
  await expect(page.getByTestId('slash-command-option-plan')).toHaveCount(1);
});

test('runtime slash commands share button state, preserve drafts, and configure the next request', async ({ page }) => {
  await page.goto('/chat/conv-slash');
  const input = page.getByTestId('chat-input-textarea');
  await input.fill('/nex');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('chat-nexus-dialog')).toBeVisible();
  await expect(input).toHaveValue('');
  await page.getByTestId('chat-nexus-confirm').click();
  await expect(page.getByTestId('chat-nexus-mode-banner')).toBeVisible();
  await input.fill('/nexus on Keep this draft');
  await page.keyboard.press('Enter');
  await expect(input).toHaveValue('Keep this draft');
  await expect(page.getByTestId('chat-nexus-dialog')).toBeHidden();
  await input.fill('/moa cross-model-code-review');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('chat-moa-mode-banner')).toContainText('Code Review');
  await input.fill('/quality code-ultra');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('chat-quality-profile-banner')).toContainText('Code Ultra');
  await input.fill('/plan');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('chat-plan-mode-banner')).toBeVisible();
  await input.fill('/normal');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('chat-plan-mode-banner')).toBeHidden();
  expect(await page.evaluate(() => (window as unknown as { __slashAgentChatCalls__: unknown[] }).__slashAgentChatCalls__.length)).toBe(0);
  await page.reload();
  await expect(page.getByTestId('chat-nexus-mode-banner')).toBeVisible();
  await expect(page.getByTestId('chat-moa-mode-banner')).toBeVisible();
  await expect(page.getByTestId('chat-quality-profile-banner')).toContainText('Code Ultra');
  await input.fill('Execute the requested work');
  await page.getByTestId('chat-send').click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __slashAgentChatCalls__: Array<Record<string, unknown>> }).__slashAgentChatCalls__[0])).toMatchObject({
    powerMode: 'nexus', collaborationMode: 'mixtureOfAgents', moaPreset: 'crossModelCodeReview', orchestrationProfile: 'codeUltra',
  });
  await expect(page.getByTestId('chat-stop')).toBeVisible();
  await input.fill('/nexus off');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('chat-nexus-mode-banner')).toBeHidden();
  await input.fill('/moa off');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('chat-moa-mode-banner')).toBeHidden();
  await input.fill('/quality deep Next draft');
  await page.keyboard.press('Enter');
  await expect(input).toHaveValue('Next draft');
  await expect(page.getByTestId('chat-quality-profile-banner')).toContainText('Deep');
  expect(await page.evaluate(() => (window as unknown as { __slashAgentChatCalls__: unknown[] }).__slashAgentChatCalls__.length)).toBe(1);
});

test('bare runtime commands and palette actions open the same pickers', async ({ page }) => {
  await page.goto('/chat/conv-slash');
  const input = page.getByTestId('chat-input-textarea');
  await input.fill('/moa');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('option', { name: 'Fast Review', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await input.fill('/quality');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('option', { name: 'Research Ultra', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await input.fill('Keep my draft');
  await page.keyboard.press('Control+Shift+P');
  const dialog = page.getByRole('dialog', { name: /command palette/i });
  await dialog.getByRole('combobox').fill('Nexus');
  await dialog.getByRole('option', { name: 'Nexus mode', exact: true }).click();
  await expect(page.getByTestId('chat-nexus-dialog')).toBeVisible();
  await expect(input).toHaveValue('Keep my draft');
});

test('invalid and unavailable local commands never reach the model', async ({ page }) => {
  await page.goto('/chat/conv-slash');
  const input = page.getByTestId('chat-input-textarea');
  for (const command of ['/nexus unknown', '/quality invalid', '/stop']) {
    await input.fill(command);
    await page.getByTestId('chat-send').click();
    await expect(input).toHaveValue(command);
  }
  await expect(page.getByTestId('chat-nexus-mode-banner')).toBeHidden();
  expect(await page.evaluate(() => (window as unknown as { __slashAgentChatCalls__: unknown[] }).__slashAgentChatCalls__.length)).toBe(0);
});

test('registered browser button automatically has a local slash command', async ({ page }) => {
  await page.goto('/chat/conv-slash');
  const input = page.getByTestId('chat-input-textarea');
  await input.fill('/browser');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('browser-dock')).toBeVisible();
  await input.fill('/browser Keep my draft');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('browser-dock')).toHaveCount(0);
  await expect(input).toHaveValue('Keep my draft');
  expect(await page.evaluate(() => (window as unknown as { __slashAgentChatCalls__: unknown[] }).__slashAgentChatCalls__.length)).toBe(0);
});

test('slash command keyboard selection scrolls with the active row', async ({ page }) => {
  await page.goto('/chat/conv-slash');

  const textarea = page.getByTestId('chat-input-textarea');
  await textarea.fill('/');

  const list = page.getByTestId('slash-command-list');
  await expect(list).toBeVisible();

  for (let i = 0; i < 13; i += 1) {
    await page.keyboard.press('ArrowDown');
  }

  await expect.poll(async () => page.evaluate(() => {
    const listEl = document.querySelector('[data-testid=\"slash-command-list\"]');
    const activeEl = document.querySelector('[data-testid=\"slash-command-list\"] [aria-selected=\"true\"]');
    if (!listEl || !activeEl) return false;
    const listRect = listEl.getBoundingClientRect();
    const activeRect = activeEl.getBoundingClientRect();
    return activeRect.top >= listRect.top - 1 && activeRect.bottom <= listRect.bottom + 1;
  })).toBe(true);
});

test('slash command menu uses localized chrome and built-in command labels', async ({ page }) => {
  await page.goto('/chat/conv-slash?locale=zh-CN');

  const textarea = page.getByTestId('chat-input-textarea');
  await textarea.fill('/plan');

  const menu = page.getByTestId('slash-command-menu');
  await expect(menu).toBeVisible();
  await expect(menu).toContainText('斜杠命令');
  await expect(menu).toContainText('规划');
  await expect(menu).toContainText('进入只读规划模式，生成可审批的实现计划。');
});

test('plan mode switch keeps its divider centered between labels', async ({ page }) => {
  await page.goto('/chat/conv-slash?locale=zh-CN');

  await expect(page.getByTestId('chat-mode-segment')).toBeVisible();

  const metrics = await page.evaluate(() => {
    const segment = document.querySelector('[data-testid="chat-mode-segment"]');
    const plan = document.querySelector('[data-testid="chat-plan-mode"]');
    const normal = document.querySelector('[data-testid="chat-normal-mode"]');
    const divider = document.querySelector('[data-testid="chat-mode-divider"]');
    if (!segment || !plan || !normal || !divider) {
      throw new Error('mode switch elements missing');
    }

    const segmentRect = segment.getBoundingClientRect();
    const planRect = plan.getBoundingClientRect();
    const normalRect = normal.getBoundingClientRect();
    const dividerRect = divider.getBoundingClientRect();

    return {
      segmentCenter: segmentRect.left + segmentRect.width / 2,
      buttonBoundary: planRect.right,
      normalLeft: normalRect.left,
      dividerCenter: dividerRect.left + dividerRect.width / 2,
    };
  });

  expect(Math.abs(metrics.buttonBoundary - metrics.normalLeft)).toBeLessThan(0.5);
  expect(Math.abs(metrics.dividerCenter - metrics.segmentCenter)).toBeLessThan(0.75);
  expect(Math.abs(metrics.dividerCenter - metrics.buttonBoundary)).toBeLessThan(1);
});

test('Nexus mode explains its cost, persists per conversation, and reaches the backend', async ({ page }) => {
  await page.goto('/chat/conv-slash');
  await page.getByTestId('chat-more-options').click();

  const nexusSwitch = page.getByTestId('chat-nexus-mode');
  await expect(nexusSwitch).toHaveAttribute('aria-pressed', 'false');
  await nexusSwitch.click();

  const dialog = page.getByTestId('chat-nexus-dialog');
  await expect(dialog).toBeVisible();
  await expect(page.getByRole('dialog', { name: 'About Nexus mode' })).toHaveCSS('opacity', '1');
  await expect(dialog).toContainText('budgets are resolved from the active endpoint and quality policy');
  await expect(dialog).toContainText('same blind spot');
  await page.getByTestId('chat-nexus-confirm').click();

  await expect(page.getByTestId('nexus-activation-effect')).toBeVisible();
  await expect(page.getByTestId('nexus-activation-effect')).toBeHidden();
  await expect(nexusSwitch).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByTestId('chat-nexus-mode-banner')).toContainText('dedicated verifier/judge lanes');

  await page.getByTestId('chat-input-textarea').fill('Review the cross-module change');
  await page.getByTestId('chat-send').click();
  await expect.poll(
    () => page.evaluate(() =>
      (window as unknown as { __slashAgentChatCalls__: Array<Record<string, unknown>> })
        .__slashAgentChatCalls__[0]?.powerMode,
    ),
  ).toBe('nexus');

  await page.reload();
  await page.getByTestId('chat-more-options').click();
  await expect(page.getByTestId('chat-nexus-mode')).toHaveAttribute('aria-pressed', 'true');
  await page.getByTestId('chat-nexus-mode').click();
  await expect(page.getByTestId('chat-nexus-mode')).toHaveAttribute('aria-pressed', 'false');

  await page.getByTestId('chat-nexus-mode').click();
  await expect(dialog).toBeHidden();
  await expect(page.getByTestId('chat-nexus-mode')).toHaveAttribute('aria-pressed', 'true');
  await page.getByTestId('chat-nexus-mode-banner').getByRole('button', { name: 'Details' }).click();
  await expect(dialog).toBeVisible();
});

test('Nexus activation respects reduced-motion preferences', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/chat/conv-slash');
  await page.getByTestId('chat-more-options').click();

  await page.getByTestId('chat-nexus-mode').click();
  await page.getByTestId('chat-nexus-confirm').click();

  await expect(page.getByTestId('chat-nexus-mode')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByTestId('nexus-activation-effect')).toHaveCount(0);
});

test('the unified quality select supports Home End Enter and Escape', async ({ page }) => {
  await page.goto('/chat/conv-slash');
  await page.getByTestId('chat-more-options').click();
  const trigger = page.getByTestId('chat-quality-profile');

  await trigger.focus();
  await trigger.press('ArrowDown');
  await page.getByRole('option').last().press('End');
  await page.getByRole('option').last().press('Enter');
  await expect(trigger).toHaveAttribute('data-value', 'custom');

  await trigger.press('ArrowDown');
  await page.getByRole('option').first().press('Home');
  await page.getByRole('option').first().press('Escape');
  await expect(trigger).toBeFocused();
});

test('MoA and orchestration profiles remain independent from Nexus and reach the backend', async ({ page }) => {
  await page.goto('/chat/conv-slash');
  await page.getByTestId('chat-more-options').click();

  await selectNexaOption(page.getByTestId('chat-moa-preset'), 'crossModelCodeReview');
  await expect(page.getByTestId('chat-moa-mode-banner')).toContainText('Code Review');
  await expect(page.getByTestId('chat-moa-mode-banner')).toContainText('Independent from Nexus');

  await selectNexaOption(page.getByTestId('chat-quality-profile'), 'codeUltra');
  await expect(page.getByTestId('chat-quality-profile-banner')).toContainText('Code Ultra');
  await expect(page.getByTestId('chat-quality-profile-banner')).toContainText('provider reasoning stays separate');

  await page.getByTestId('chat-nexus-mode').click();
  await page.getByTestId('chat-nexus-confirm').click();
  await expect(page.getByTestId('chat-moa-mode-banner')).toContainText('Nexus + MoA');

  await page.getByTestId('chat-input-textarea').fill('Review and verify the implementation');
  await page.getByTestId('chat-send').click();
  await expect.poll(
    () => page.evaluate(() => {
      const request = (window as unknown as { __slashAgentChatCalls__: Array<Record<string, unknown>> })
        .__slashAgentChatCalls__[0];
      return [
        request?.powerMode,
        request?.collaborationMode,
        request?.moaPreset,
        request?.orchestrationProfile,
      ];
    }),
  ).toEqual(['nexus', 'mixtureOfAgents', 'crossModelCodeReview', 'codeUltra']);

  await page.reload();
  await page.getByTestId('chat-more-options').click();
  await expect(page.getByTestId('chat-moa-mode-banner')).toContainText('Nexus + MoA');
  await page.getByTestId('chat-nexus-mode').click();
  await expect(page.getByTestId('chat-nexus-mode')).toHaveAttribute('aria-pressed', 'false');
  await expect(page.getByTestId('chat-moa-mode-banner')).toBeVisible();
});

test('Custom orchestration exposes bounded runtime controls', async ({ page }) => {
  await page.goto('/chat/conv-slash');
  await page.getByTestId('chat-more-options').click();
  await selectNexaOption(page.getByTestId('chat-quality-profile'), 'custom');
  await page.getByTestId('chat-quality-custom-maxIterations').fill('48');
  await page.getByTestId('chat-quality-custom-maxParallel').fill('8');
  await page.getByTestId('chat-quality-custom-maxCallsPerTurn').fill('10');
  await page.getByTestId('chat-quality-custom-delegatedTokenBudget').fill('96000');
  await page.getByTestId('chat-quality-custom-retryLimit').fill('3');
  await page.getByTestId('chat-quality-custom-minEvidenceSources').fill('4');
  await page.getByTestId('chat-quality-custom-verificationReservePercent').fill('40');
  await page.getByTestId('chat-input-textarea').fill('Run a custom verified workflow');
  await page.getByTestId('chat-send').click();

  await expect.poll(
    () => page.evaluate(() => {
      const request = (window as unknown as { __slashAgentChatCalls__: Array<Record<string, unknown>> })
        .__slashAgentChatCalls__[0];
      return request?.customOrchestration;
    }),
  ).toMatchObject({
    maxIterations: 48,
    maxParallel: 8,
    maxCallsPerTurn: 10,
    delegatedTokenBudget: 96000,
    retryLimit: 3,
    minEvidenceSources: 4,
    verificationReservePercent: 40,
  });
});
