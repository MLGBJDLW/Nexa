import { expect, test } from './timeline-test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale', 'en');

    const nowIso = new Date().toISOString();
    const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

    const conversation = {
      id: 'conv-workflows',
      title: 'Workflow catalog',
      provider: 'open_ai',
      model: 'gpt-4.1',
      systemPrompt: '',
      collectionContext: null,
      projectId: null,
      createdAt: nowIso,
      updatedAt: nowIso,
    };

    const defaultAgentConfig = {
      id: 'cfg-workflows',
      name: 'Workflow Config',
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

    const workflowCatalog = [
      {
        id: 'research_verify',
        label: 'Research + Verify',
        description: 'Parallel evidence gathering, verification, and critique.',
        maxParallel: 3,
        promptTemplate: 'Run the Research + Verify workflow for this goal:\n\nGoal:\n',
        tasks: [
          {
            id: 'research',
            roleId: 'researcher',
            roleLabel: 'Researcher',
            task: 'Gather evidence.',
            expectedOutput: 'Evidence-backed findings.',
            deliverableStyle: 'research brief',
            acceptanceCriteria: ['Use retrieval before concluding.'],
          },
        ],
      },
      {
        id: 'meeting_summary',
        label: 'Meeting Summary',
        description: 'Turn notes into decisions, actions, risks, and follow-ups.',
        maxParallel: 3,
        promptTemplate: 'Run the Meeting Summary workflow for this material:\n\nMaterial:\n',
        tasks: [
          {
            id: 'extract',
            roleId: 'researcher',
            roleLabel: 'Researcher',
            task: 'Extract decisions and actions.',
            expectedOutput: 'Meeting facts.',
            deliverableStyle: 'meeting brief',
            acceptanceCriteria: ['Separate explicit decisions from inferred actions.'],
          },
        ],
      },
    ];

    const callbackMap = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, { event: string; handlerId: number }>();
    let callbackSeq = 1;
    let listenerSeq = 1;
    const taskSummaryReads: unknown[] = [];
    (window as unknown as { __taskSummaryReads: unknown[] }).__taskSummaryReads = taskSummaryReads;

    const invoke = async (cmd: string, args: Record<string, unknown> = {}) => {
      switch (cmd) {
        case 'list_agent_task_run_summaries_cmd':
          taskSummaryReads.push(args);
          return { items: [], nextCursor: null };
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
        case 'list_workflow_templates_cmd':
          return clone(workflowCatalog);
        case 'list_agent_configs_cmd':
          return [clone(defaultAgentConfig)];
        case 'get_model_context_window':
          return 1047576;
        case 'get_wizard_state_cmd':
          return { completed: true, language: 'en', aiProvider: 'open_ai', sourceAdded: true };
        case 'list_conversations_cmd':
          return [clone(conversation)];
        case 'get_conversation_cmd':
          return [clone(conversation), []];
        case 'get_conversation_turns_cmd':
        case 'get_agent_task_runs_cmd':
        case 'list_sources':
        case 'get_conversation_sources_cmd':
        case 'list_checkpoints_cmd':
        case 'list_user_memories_cmd':
        case 'list_skills_cmd':
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

for (const width of [1360, 360]) {
for (const route of ['/chat', '/chat/conv-workflows']) {
test(`chat task tools open on demand at ${width}px on ${route}`, async ({ page }) => {
  await page.setViewportSize({ width, height: 900 });
  await page.goto(route);
  await expect(page.getByTestId('chat-input-textarea')).toBeVisible();
  await expect(page.getByTestId('task-home')).toHaveCount(0);
  await expect(page.getByTestId('chat-task-tools-trigger')).toBeHidden();
  await page.screenshot({ path: `../../.artifacts/chat-task-quickfix/${route === '/chat' ? 'new' : 'existing'}-chat-${width}.png`, animations: 'disabled' });
  expect(await page.evaluate(() => (window as unknown as { __taskSummaryReads: unknown[] }).__taskSummaryReads)).toEqual([]);
  await page.getByTestId('chat-more-options').click();
  await expect(page.getByTestId('task-home')).toHaveCount(0);
  await page.getByTestId('chat-task-tools-trigger').click();
  await expect(page.getByTestId('task-home')).toBeVisible();
  const panel = page.getByTestId('chat-task-tools-panel');
  await expect(panel).toBeVisible();
  const bounds = await panel.evaluate(element => {
    const rect = element.getBoundingClientRect();
    return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom, overlay: !!element.closest('[data-nexa-overlay-root="true"]') };
  });
  expect(bounds.overlay).toBe(true);
  expect(bounds.left).toBeGreaterThanOrEqual(0);
  expect(bounds.right).toBeLessThanOrEqual(width);
  expect(bounds.top).toBeGreaterThanOrEqual(0);
  expect(bounds.bottom).toBeLessThanOrEqual(900);
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('task-home')).toHaveCount(0);
  await page.getByTestId('chat-more-options').click();
  const input = page.getByTestId('chat-input-textarea');
  await input.fill('/task-center');
  await input.press('Enter');
  await expect(panel).toBeVisible();
  await expect(input).toHaveValue('');
  await page.screenshot({ path: `../../.artifacts/chat-task-quickfix/task-tools-${width}.png`, animations: 'disabled' });
  await page.keyboard.press('Escape');
  await input.fill('/workflow');
  await input.press('Enter');
  await expect(page.getByTestId('workflow-catalog-panel')).toBeVisible();
  await expect(input).toHaveValue('');
});
}
}

test('workflow catalog can prefill the chat composer', async ({ page }) => {
  await page.goto('/chat/conv-workflows');

  const primaryControls = page.getByTestId('chat-composer-primary-controls');
  const modeSegment = page.getByTestId('chat-mode-segment');
  const workflowTrigger = page.getByTestId('workflow-catalog-trigger');
  const attachmentTrigger = page.getByTestId('chat-attachment-trigger');
  await expect(primaryControls).toBeVisible();
  await expect(primaryControls.getByTestId('chat-mode-segment')).toBeVisible();
  await expect(page.getByTestId('chat-input-toolbar').getByTestId('chat-mode-segment')).toHaveCount(0);
  await expect(attachmentTrigger).toBeVisible();
  await expect(modeSegment).toBeVisible();
  await expect(workflowTrigger).toBeHidden();
  await page.getByTestId('chat-more-options').click();
  await expect(page.getByTestId('chat-advanced-options').getByTestId('workflow-catalog-trigger')).toBeVisible();

  await workflowTrigger.click();
  const catalog = page.getByTestId('workflow-catalog-panel');

  await expect(catalog).toBeVisible();
  await expect(catalog).toContainText('Research + Verify');
  await expect(catalog).toContainText('Meeting Summary');

  const overlayPlacement = await catalog.evaluate((panel) => {
    const rect = panel.getBoundingClientRect();
    return {
      inOverlayRoot: Boolean(panel.closest('[data-nexa-overlay-root="true"]')),
      left: rect.left,
      right: rect.right,
      viewportWidth: window.innerWidth,
    };
  });
  expect(overlayPlacement.inOverlayRoot).toBe(true);
  expect(overlayPlacement.left).toBeGreaterThanOrEqual(0);
  expect(overlayPlacement.right).toBeLessThanOrEqual(overlayPlacement.viewportWidth);

  await catalog.getByRole('button', { name: /Meeting Summary/ }).click();

  const composer = page.getByTestId('chat-input-textarea');
  await expect(composer).toHaveValue(/spawn_subagent_batch/);
  await expect(composer).toHaveValue(/workflow_template: meeting_summary/);
  await expect(composer).toHaveValue(/batch_goal:/);
  await expect(composer).toHaveValue(/Run the Meeting Summary workflow/);
  await expect(catalog).toHaveCount(0);
});

test('composer primary controls stay compact at narrow widths', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 780 });
  await page.goto('/chat/conv-workflows');

  const controls = page.getByTestId('chat-composer-primary-controls');
  await expect(page.getByTestId('workflow-catalog-trigger')).toBeHidden();
  await expect(page.getByTestId('chat-attachment-trigger')).toBeVisible();
  const geometry = await controls.evaluate((element) => {
    const children = Array.from(element.children).map((child) => {
      const rect = child.getBoundingClientRect();
      return rect.y + rect.height / 2;
    });
    return {
      clientWidth: element.clientWidth,
      scrollWidth: element.scrollWidth,
      rowSpread: children.length ? Math.max(...children) - Math.min(...children) : 0,
    };
  });
  expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.clientWidth);
  expect(geometry.rowSpread).toBeLessThanOrEqual(2);
});
