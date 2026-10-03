import { expect, test } from './timeline-test';

test('chat worktree keeps the draft and requires a snapshot decision before archive', async ({ page }, testInfo) => {
  await page.goto('/chat/conv-active'); await page.getByTestId('chat-input-textarea').waitFor();
  await page.evaluate(() => {
    const state = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> }; __worktreeActions: string[] };
    const original = state.__TAURI_INTERNALS__.invoke; state.__worktreeActions = [];
    let record: Record<string, unknown> | null = null;
    state.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === 'get_chat_worktree_cmd') return record;
      if (command === 'change_chat_worktree_cmd') {
        const action = String(args?.action); state.__worktreeActions.push(action);
        record = { id: 'worktree-one', conversationId: 'conv-active', path: 'D:/Nexa/chat-worktrees/one', branch: 'nexa/chat-one', startSha: 'a'.repeat(40), status: action === 'archive' ? 'archived' : 'ready', snapshotSha: action === 'create' ? null : 'b'.repeat(40), detail: null };
        return record;
      }
      return original(command, args);
    };
  });
  const input = page.getByTestId('chat-input-textarea'); await input.fill('Keep my worktree draft');
  await page.keyboard.press('Control+Shift+P'); const palette = page.getByRole('dialog', { name: /Command Palette/i });
  await palette.getByRole('combobox').fill('Chat worktree'); await palette.getByRole('option', { name: 'Chat worktree', exact: true }).click();
  const panel = page.getByTestId('chat-worktree-panel'); await expect(panel).toBeVisible();
  await panel.getByRole('textbox', { name: 'Starting reference' }).fill('main');
  await panel.getByRole('button', { name: 'Create worktree', exact: true }).click();
  await expect(panel).toContainText('nexa/chat-one');
  await expect(panel.getByRole('button', { name: 'Archive with snapshot' })).toBeDisabled();
  await panel.getByRole('checkbox').check();
  await page.screenshot({ path: testInfo.outputPath('chat-worktree.png') });
  await panel.getByRole('button', { name: 'Archive with snapshot' }).click();
  await expect(panel).toContainText('archived');
  await panel.getByRole('button', { name: 'Restore snapshot' }).click(); await expect(panel).toContainText('ready');
  await page.keyboard.press('Escape'); await expect(input).toHaveValue('Keep my worktree draft');
  await input.fill('/worktree'); await page.keyboard.press('Enter'); await expect(panel).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __worktreeActions: string[] }).__worktreeActions)).toEqual(['create', 'archive', 'restore']);
});

test('MCP resources and prompt templates are reviewed before adding text to the draft', async ({ page }, testInfo) => {
  await page.goto('/chat/conv-active'); await page.getByTestId('chat-input-textarea').waitFor();
  await page.evaluate(() => {
    const state = window as unknown as { __TAURI_INTERNALS__: { invoke: (command:string,args?:Record<string,unknown>) => Promise<unknown> }; __mcpReads:Array<Record<string,unknown>> };
    const original = state.__TAURI_INTERNALS__.invoke; state.__mcpReads = [];
    state.__TAURI_INTERNALS__.invoke = async (command,args) => {
      if (command === 'list_mcp_servers_cmd') return [{ id:'knowledge',name:'Knowledge',enabled:true }];
      if (command === 'get_mcp_content_catalog_cmd') return { authorityEpoch:7,complete:true,diagnostics:null,resources:[{ name:'Report',uri:'notes://report' }],resourceTemplates:[{name:'Note',uriTemplate:'notes://{id}'}],prompts:[{name:'summarize',arguments:[{name:'topic',required:true}]}] };
      if (command === 'read_mcp_content_cmd') {
        state.__mcpReads.push(args!); const request=args?.request as { action:string };
        const content=request.action === 'read_resource' ? 'Quarterly report evidence' : 'Selected prompt template';
        return { content,isError:false,artifacts:{ kind:'mcpToolResult',version:1,contentBlocks:[{type:'text',text:content}],notices:[] } };
      }
      return original(command,args);
    };
  });
  const input=page.getByTestId('chat-input-textarea'); await input.fill('Keep my draft');
  await page.keyboard.press('Control+Shift+P'); const palette=page.getByRole('dialog',{name:/command palette/i});
  await palette.getByRole('combobox').fill('MCP resources'); await palette.getByRole('option',{name:'MCP resources and prompts',exact:true}).click();
  const panel=page.getByTestId('mcp-content-panel'); await panel.getByRole('combobox',{name:'Connector',exact:true}).selectOption('knowledge');
  await panel.getByRole('combobox',{name:'Resources',exact:true}).selectOption('notes://report');
  expect(await page.evaluate(() => (window as unknown as {__mcpReads:unknown[]}).__mcpReads)).toEqual([]);
  await panel.getByRole('button',{name:'Read content',exact:true}).click(); await expect(panel).toContainText('Quarterly report evidence');
  await page.screenshot({path:testInfo.outputPath('mcp-resources.png')});
  await panel.getByRole('button',{name:'Add text to draft'}).click(); await expect(panel).toBeHidden(); await expect(input).toHaveValue(/Keep my draft\n\nMCP · Knowledge · notes:\/\/report\nQuarterly report evidence/);
  await input.fill('/mcp-context'); await page.keyboard.press('Enter'); await expect(panel).toBeVisible();
  await panel.getByRole('combobox',{name:'Connector',exact:true}).selectOption('knowledge');
  await panel.getByRole('combobox',{name:'Content type',exact:true}).selectOption('prompt');
  await panel.getByRole('combobox',{name:'Prompt templates',exact:true}).selectOption('summarize');
  await expect(panel.getByRole('button',{name:'Read content',exact:true})).toBeDisabled();
  await panel.getByRole('textbox',{name:'topic *',exact:true}).fill('quarter');
  await panel.getByRole('button',{name:'Read content',exact:true}).click(); await expect(panel).toContainText('Selected prompt template');
  const reads=await page.evaluate(() => (window as unknown as {__mcpReads:Array<{authorityEpoch:number;request:{action:string;arguments?:Record<string,string>}}>}).__mcpReads);
  expect(reads.map(read => read.authorityEpoch)).toEqual([7,7]); expect(reads[1].request.arguments).toEqual({topic:'quarter'});
});

test('project checks require explicit enablement and expose receipts through hooks command', async ({ page }, testInfo) => {
  await page.goto('/chat/conv-active');
  await page.getByTestId('chat-input-textarea').waitFor();
  await page.evaluate(() => {
    const state = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> }; __hookSaves: unknown[] };
    state.__hookSaves = [];
    const original = state.__TAURI_INTERNALS__.invoke;
    let hooks: Array<Record<string, unknown>> = [];
    state.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === 'get_project_hooks_cmd') return {
        hooks, runs: hooks.length ? [{ id:'run-1',event:'before_complete',status:'failed',detail:'check: test assertions failed',createdAt:'2026-10-03 08:00:00' }] : [],
        catalog: { errors:[],tools:[{ name:'check',manifestHash:'a'.repeat(64),commandPreview:'npm test',runnable:true,warnings:[] }] },
      };
      if (command === 'save_project_hook_cmd') {
        state.__hookSaves.push(args);
        const hook = { ...(args?.hook as Record<string, unknown>),id:'hook-1' }; hooks = [hook]; return hook;
      }
      if (command === 'delete_project_hook_cmd') { hooks = []; return null; }
      return original(command, args);
    };
  });
  const input = page.getByTestId('chat-input-textarea');
  await input.fill('Keep this draft');
  await page.keyboard.press('Control+Shift+P');
  const palette = page.getByRole('dialog', { name:/command palette/i });
  await palette.getByRole('combobox').fill('Project checks');
  await palette.getByRole('option', { name:'Project checks',exact:true }).click();
  const panel = page.getByTestId('project-hooks-panel');
  await panel.getByRole('combobox', { name:'Project tool',exact:true }).selectOption(`check:${'a'.repeat(64)}`);
  await expect(panel).toContainText('npm test');
  expect(await page.evaluate(() => (window as unknown as { __hookSaves: unknown[] }).__hookSaves)).toEqual([]);
  await panel.getByRole('button', { name:'Enable check',exact:true }).click();
  await expect(panel.getByRole('button', { name:'Disable',exact:true })).toBeVisible();
  await panel.locator('summary').click();
  await expect(panel).toContainText('check: test assertions failed');
  await page.screenshot({ path:testInfo.outputPath('project-checks.png') });
  await panel.getByRole('button', { name:'Disable',exact:true }).click();
  await expect(panel).toContainText('Disabled');
  await page.keyboard.press('Escape'); await expect(input).toHaveValue('Keep this draft');
  await input.fill('/hooks'); await page.keyboard.press('Enter'); await expect(panel).toBeVisible();
  const saves = await page.evaluate(() => (window as unknown as { __hookSaves:Array<{ projectId:string;hook:{enabled:boolean;manifestHash:string;event:string} }> }).__hookSaves);
  expect(saves.map(save => save.hook.enabled)).toEqual([true,false]);
  expect(saves.every(save => save.projectId === 'project-legacy' && save.hook.manifestHash === 'a'.repeat(64) && save.hook.event === 'before_complete')).toBe(true);
});

test('project file rules share a local command and inspector without changing the draft', async ({ page }, testInfo) => {
  await page.goto('/chat/conv-active');
  await page.getByTestId('chat-input-textarea').waitFor();
  await page.evaluate(() => {
    const state = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> }; __ruleRequests: unknown[] };
    state.__ruleRequests = [];
    const original = state.__TAURI_INTERNALS__.invoke;
    state.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command !== 'get_project_rules_cmd') return original(command, args);
      state.__ruleRequests.push(args);
      const child = args?.path === 'src/example.ts';
      return { revision: child ? 'child-snapshot' : 'root-snapshot', diagnostics: [], files: [{
        path: child ? 'D:/Project/src/AGENTS.md' : 'D:/Project/AGENTS.md',
        scope: child ? 'D:/Project/src' : 'D:/Project',
        revision: child ? 'updated-child-revision' : 'root-revision',
        content: child ? 'Run the frontend verification.' : 'Preserve manually entered report cells.',
        truncated: false,
      }] };
    };
  });
  const input = page.getByTestId('chat-input-textarea');
  await input.fill('Keep my draft');
  await page.keyboard.press('Control+Shift+P');
  const palette = page.getByRole('dialog', { name: /command palette/i });
  await palette.getByRole('combobox').fill('File rules');
  await palette.getByRole('option', { name: 'File rules', exact: true }).click();
  const panel = page.getByTestId('workspace-rules-panel');
  await expect(panel).toBeVisible();
  await panel.locator('summary').click();
  await expect(panel).toContainText('Preserve manually entered report cells.');
  await panel.getByRole('textbox', { name: 'File or folder path (optional)' }).fill('src/example.ts');
  await panel.getByRole('button', { name: 'Refresh rules' }).click();
  await expect(panel.locator('summary')).toHaveText('D:/Project/src/AGENTS.md');
  await panel.locator('summary').click();
  await expect(panel).toContainText('Run the frontend verification.');
  await page.keyboard.press('Escape');
  await expect(panel).toBeHidden();
  await expect(input).toHaveValue('Keep my draft');
  await input.fill('/rules');
  await page.keyboard.press('Enter');
  await expect(panel).toBeVisible();
  await expect(panel.locator('summary')).toHaveText('D:/Project/AGENTS.md');
  const requests = await page.evaluate(() => (window as unknown as { __ruleRequests: Array<{ projectId: string; conversationId: string | null; path: string | null }> }).__ruleRequests);
  expect(requests.every(request => request.projectId === 'project-legacy' && request.conversationId === 'conv-active')).toBe(true);
  expect(requests.some(request => request.path === 'src/example.ts')).toBe(true);
  await panel.locator('summary').click();
  await page.screenshot({ path: testInfo.outputPath('workspace-file-rules.png') });
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale', 'en');
    localStorage.setItem('active-project-id', 'project-legacy');

    const nowIso = new Date().toISOString();
    const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;
    const activeConversation = {
      id: 'conv-active',
      title: 'Active conversation',
      provider: 'open_ai',
      model: 'gpt-4.1',
      systemPrompt: 'Legacy copied project prompt',
      collectionContext: null,
      projectId: 'project-legacy',
      personaId: 'programmer',
      initialAutoTitlePending: false,
      archivedAt: null,
      createdAt: nowIso,
      updatedAt: nowIso,
    };
    const archivedConversation = {
      ...activeConversation,
      id: 'conv-archived',
      title: 'Archived conversation',
      archivedAt: nowIso,
    };
    const archivedMessages = [
      {
        id: 'msg-archived-user',
        conversationId: 'conv-archived',
        role: 'user',
        content: 'Keep this archived question available.',
        toolCallId: null,
        toolCalls: [],
        artifacts: null,
        tokenCount: 8,
        createdAt: nowIso,
        sortOrder: 0,
        thinking: null,
        imageAttachments: null,
      },
      {
        id: 'msg-archived-assistant',
        conversationId: 'conv-archived',
        role: 'assistant',
        content: 'This archived answer remains readable.',
        toolCallId: null,
        toolCalls: [],
        artifacts: null,
        tokenCount: 8,
        createdAt: nowIso,
        sortOrder: 1,
        thinking: null,
        imageAttachments: null,
      },
    ];
    let active = [activeConversation];
    let archived = [archivedConversation];
    const projects = [{
      id: 'project-legacy', name: 'Legacy project', description: '', icon: 'folder',
      color: '#3b82f6', systemPrompt: 'Current live project prompt', sourceScope: null,
      archived: false, createdAt: nowIso, updatedAt: nowIso,
    }];
    const commands: string[] = [];
    const createConversationArgs: Array<Record<string, unknown>> = [];

    const defaultAgentConfig = {
      id: 'cfg-archive',
      name: 'Archive Config',
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

    const callbackMap = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, { event: string; handlerId: number }>();
    let callbackSeq = 1;
    let listenerSeq = 1;

    const invoke = async (cmd: string, args: Record<string, unknown> = {}) => {
      commands.push(cmd);
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
        case 'list_agent_configs_cmd':
          return [clone(defaultAgentConfig)];
        case 'get_model_context_window':
          return 1047576;
        case 'get_wizard_state_cmd':
          return { completed: true, language: 'en', aiProvider: 'open_ai', sourceAdded: true };
        case 'list_conversations_cmd':
          return clone(active);
        case 'create_conversation_cmd': {
          createConversationArgs.push(clone(args));
          if (localStorage.getItem('e2e-fail-next-conversation-create') === '1') {
            localStorage.removeItem('e2e-fail-next-conversation-create');
            throw new Error('Injected conversation create failure');
          }
          if (localStorage.getItem('e2e-delay-conversation-create') === '1') {
            await new Promise((resolve) => setTimeout(resolve, 300));
          }
          const conversation = {
            ...activeConversation,
            id: 'conv-new',
            title: '',
            systemPrompt: String(args.systemPrompt ?? ''),
            projectId: args.projectId == null ? null : String(args.projectId),
            personaId: args.personaId == null ? null : String(args.personaId),
          };
          active = [conversation, ...active];
          return clone(conversation);
        }
        case 'list_archived_conversations_cmd':
          return clone(archived);
        case 'archive_conversation_cmd': {
          const id = String(args.id ?? '');
          const conversation = active.find((item) => item.id === id);
          if (!conversation) return null;
          active = active.filter((item) => item.id !== id);
          const next = { ...conversation, archivedAt: new Date().toISOString() };
          archived = [next, ...archived];
          return clone(next);
        }
        case 'unarchive_conversation_cmd': {
          const id = String(args.id ?? '');
          const conversation = archived.find((item) => item.id === id);
          if (!conversation) return null;
          archived = archived.filter((item) => item.id !== id);
          const next = { ...conversation, archivedAt: null };
          active = [next, ...active];
          return clone(next);
        }
        case 'delete_conversation_cmd': {
          const id = String(args.id ?? '');
          if (localStorage.getItem('e2e-delay-conversation-delete') === '1') {
            localStorage.removeItem('e2e-delay-conversation-delete');
            await new Promise((resolve) => setTimeout(resolve, 750));
          }
          active = active.filter((item) => item.id !== id);
          archived = archived.filter((item) => item.id !== id);
          return null;
        }
        case 'agent_chat_cmd':
          if (localStorage.getItem('e2e-fail-next-agent-launch') === '1') {
            localStorage.removeItem('e2e-fail-next-agent-launch');
            throw new Error('Injected agent launch failure');
          }
          if (localStorage.getItem('e2e-delay-next-agent-launch') === '1') {
            localStorage.removeItem('e2e-delay-next-agent-launch');
            await new Promise((resolve) => setTimeout(resolve, 300));
          }
          return null;
        case 'get_conversation_cmd': {
          const id = String(args.id ?? '');
          const conversation = [...active, ...archived].find((item) => item.id === id);
          return [
            clone(conversation),
            id === 'conv-archived' ? clone(archivedMessages) : [],
          ];
        }
        case 'get_conversation_turns_cmd':
        case 'get_agent_task_runs_cmd':
        case 'list_sources':
        case 'get_conversation_sources_cmd':
        case 'list_user_memories_cmd':
        case 'list_skills_cmd':
        case 'list_mcp_servers_cmd':
        case 'list_checkpoints_cmd':
        case 'list_project_memories_cmd':
          return [];
        case 'list_personas_cmd':
          return [
            {
              id: 'default',
              name: 'Default',
              description: 'Balanced assistant',
              instructions: '',
              enabled: true,
              builtin: true,
              defaultSkillIds: [],
              createdAt: nowIso,
              updatedAt: nowIso,
            },
            {
              id: 'programmer',
              name: 'Programmer',
              description: 'Software engineering assistant',
              instructions: 'Act as a programmer.',
              enabled: true,
              builtin: true,
              defaultSkillIds: [],
              createdAt: nowIso,
              updatedAt: nowIso,
            },
          ];
        case 'list_projects_cmd':
          return clone(projects);
        case 'get_project_cmd':
          return clone(projects.find(project => project.id === args.id));
        case 'update_project_cmd': {
          const input = args.input as Record<string, unknown>;
          const index = projects.findIndex(project => project.id === args.id);
          projects[index] = { ...projects[index], ...input };
          (window as any).__PROJECT_UPDATE_INPUT__ = clone(input);
          return clone(projects[index]);
        }
        case 'create_project_cmd': {
          const input = args.input as { name: string; workspaceRoots?: string[] };
          (window as any).__PROJECT_CREATE_INPUT__ = clone(input);
          const project = { ...projects[0], ...input, id: `project-${projects.length}` };
          projects.push(project);
          return clone(project);
        }
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

    (window as unknown as { __ARCHIVE_COMMANDS__: string[] }).__ARCHIVE_COMMANDS__ = commands;
    (window as unknown as { __CREATE_CONVERSATION_ARGS__: Array<Record<string, unknown>> })
      .__CREATE_CONVERSATION_ARGS__ = createConversationArgs;
    (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
      invoke,
      metadata: { currentWindow: { label: 'main' } },
      transformCallback: (callback: (event: unknown) => void) => {
        const id = callbackSeq++;
        callbackMap.set(id, callback);
        return id;
      },
      unregisterCallback: (id: number) => callbackMap.delete(id),
      convertFileSrc: (filePath: string) => filePath,
    };
    (window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__: unknown }).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: (_event: string, eventId: number) => listeners.delete(eventId),
    };
  });
});

test('conversation actions offer archive and delete with reversible archive', async ({ page }) => {
  await page.goto('/chat/conv-active');

  await page.getByTestId('conversation-item-conv-active').hover();
  await page.getByTestId('conversation-actions-trigger-conv-active').click();
  const actions = page.getByTestId('conversation-actions-conv-active');
  await expect(actions.getByRole('button', { name: 'Archive' })).toBeVisible();
  await expect(actions.getByRole('button', { name: 'Delete' })).toBeVisible();

  await actions.getByRole('button', { name: 'Archive' }).click();
  await expect(page.getByTestId('conversation-item-conv-active')).toBeHidden();
  await page.getByRole('button', { name: 'Undo' }).click();
  await expect(page.getByTestId('conversation-item-conv-active')).toBeVisible();
});

test('conversation quick actions remain individually clickable', async ({ page }, testInfo) => {
  await page.goto('/chat/conv-active');
  const item = page.getByTestId('conversation-item-conv-active');

  expect(await item.evaluate((element) => ({
    rowIsSyntheticButton: element.getAttribute('role') === 'button',
    containsNestedButton: element.querySelector('button') !== null,
  }))).toEqual({
    rowIsSyntheticButton: false,
    containsNestedButton: true,
  });
  await expect(item.getByTestId('conversation-select-conv-active')).toHaveAttribute('type', 'button');
  await page.screenshot({ path: testInfo.outputPath('compact-conversation-sidebar.png') });

  await item.hover();
  const pin = item.getByRole('button', { name: 'Pinned' });
  const edit = item.getByRole('button', { name: 'Edit' });
  const move = item.getByRole('button', { name: 'Move to project' });
  for (const button of [pin, edit, move]) {
    await expect(button).toBeVisible();
    await expect(button).toBeEnabled();
    expect(await button.evaluate((element) => {
      const rect = element.getBoundingClientRect();
      const hit = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
      return hit?.closest('button') === element;
    })).toBe(true);
  }

  await pin.click();
  await expect.poll(() => page.evaluate(() =>
    JSON.parse(localStorage.getItem('chat-pinned-conversations') ?? '[]') as string[],
  )).toContain('conv-active');

  await item.hover();
  await item.getByRole('button', { name: 'Edit' }).click();
  await expect(item.getByRole('textbox')).toBeVisible();
  await item.getByRole('button', { name: 'Cancel' }).click();

  await item.hover();
  await item.getByRole('button', { name: 'Move to project' }).click();
  const moveMenu = page.getByTestId('conversation-move-menu');
  await expect(moveMenu.getByText('Move to Project', { exact: true })).toBeVisible();
  await expect(moveMenu.getByRole('menuitem', { name: 'Legacy project' })).toBeVisible();
  expect(await moveMenu.evaluate((element) => element.parentElement === document.body)).toBe(true);
});

test('archive feedback remains an overlay and never participates in the app layout', async ({ page }) => {
  await page.goto('/chat/conv-active');

  const appMain = page.locator('main');
  const before = await appMain.boundingBox();
  if (!before) throw new Error('app main layout is not measurable');

  const removedSonnerStyles = await page.evaluate(() => {
    const styles = Array.from(document.querySelectorAll('style'));
    const sonnerStyles = styles.filter((style) => style.textContent?.includes('[data-sonner-toaster]'));
    sonnerStyles.forEach((style) => style.remove());
    return sonnerStyles.length;
  });
  expect(removedSonnerStyles).toBeGreaterThan(0);

  await page.getByTestId('conversation-item-conv-active').hover();
  await page.getByTestId('conversation-actions-trigger-conv-active').click();
  await page.getByTestId('conversation-actions-conv-active').getByRole('button', { name: 'Archive' }).click();
  await expect(page.getByRole('button', { name: 'Undo' })).toBeVisible();

  const notificationLayout = await page.locator('[data-sonner-toaster]').evaluate((toaster) => {
    const rect = toaster.getBoundingClientRect();
    const style = getComputedStyle(toaster);
    return {
      position: style.position,
      right: window.innerWidth - rect.right,
      bottom: window.innerHeight - rect.bottom,
      portaledToBody: toaster.closest('section')?.parentElement === document.body,
    };
  });
  const after = await appMain.boundingBox();
  if (!after) throw new Error('app main layout disappeared after archive feedback');

  expect(notificationLayout).toMatchObject({ position: 'fixed', portaledToBody: true });
  expect(notificationLayout.right).toBeGreaterThanOrEqual(0);
  expect(notificationLayout.bottom).toBeGreaterThanOrEqual(0);
  expect(after).toEqual(before);
});

test('project editor stays above the glass sidebar and covers the viewport', async ({ page }, testInfo) => {
  await page.addInitScript(() => {
    const plugin = {
      manifestVersion: 2, kind: 'theme-resource', id: 'project-glass', name: 'Project Glass',
      theme: {
        baseTheme: 'light', mode: 'light',
        colors: {
          surface0: '#fff8f5', surface1: '#fff5f4', surface2: '#f9e8e8',
          surface3: '#efdada', surface4: '#e6cdcd', textPrimary: '#35252c',
          textSecondary: '#60424d', textTertiary: '#865c6b', accent: '#ce7292',
        },
        effects: { surfaceOpacity: 0.37, glassBlur: 23 },
        typography: {}, motion: {}, brand: {}, content: {}, components: {},
        background: {
          kind: 'gradient', value: 'linear-gradient(135deg, #dec2cd, #f4e8da)',
          opacity: 1, dim: 0.1, overlayColor: '#372932',
        },
      },
    };
    localStorage.setItem('nexa-theme-resource-plugins-v2', JSON.stringify([plugin]));
    localStorage.setItem('nexa-active-theme-v1', plugin.id);
  });
  await page.goto('/chat/conv-active');
  await expect(page.locator('html')).toHaveAttribute('data-theme-backdrop', 'true');
  const sidebar = page.getByTestId('chat-history-sidebar');
  await sidebar.getByRole('button', { name: 'Legacy project', exact: true }).click();
  await sidebar.getByRole('button', { name: 'New Project', exact: true }).click();
  const editor = page.getByRole('dialog');
  await expect(editor).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('project-glass-wide.png') });
  const bounds = await editor.boundingBox();
  expect(bounds!.width).toBeGreaterThan(550);
  expect(Math.abs(bounds!.x + bounds!.width / 2 - page.viewportSize()!.width / 2)).toBeLessThan(2);
  await expect(page.getByPlaceholder('Enter project name...')).toBeFocused();
  await expect(page.getByTestId('modal-viewport')).toHaveCSS('width', '1360px');
  await expect(page.getByTestId('modal-viewport')).toHaveCSS('height', '900px');
  await page.getByPlaceholder('Enter project name...').pressSequentially('Glass project');
  await expect(page.getByPlaceholder('Enter project name...')).toHaveValue('Glass project');
  await page.getByLabel('Primary folder', { exact: true }).fill('D:/work/glass-project');
  for (const viewport of [{ width: 760, height: 700 }, { width: 580, height: 460 }]) {
    await page.setViewportSize(viewport);
    const rect = await editor.boundingBox();
    expect(rect!.x).toBeGreaterThanOrEqual(8);
    expect(rect!.y).toBeGreaterThanOrEqual(8);
    expect(rect!.x + rect!.width).toBeLessThanOrEqual(viewport.width - 8);
    expect(rect!.y + rect!.height).toBeLessThanOrEqual(viewport.height - 8);
    await expect(page.getByTestId('project-save')).toBeInViewport();
    expect(await page.getByTestId('project-save').evaluate((element) => {
      const rect = element.getBoundingClientRect();
      return element.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
    })).toBe(true);
  }
  await page.screenshot({ path: testInfo.outputPath('project-glass-compact.png') });
  await page.getByTestId('project-save').click();
  await expect(editor).toBeHidden();
  await expect(page.getByTestId('project-new-conversation')).toContainText('Glass project');
  await page.setViewportSize({ width: 1360, height: 900 });
  const assertCentered = async () => {
    await expect(editor).toBeVisible();
    const box = await editor.boundingBox();
    expect(Math.abs(box!.x + box!.width / 2 - 680)).toBeLessThan(2);
    await expect(page.getByTestId('modal-viewport')).toHaveCSS('width', '1360px');
  };
  await sidebar.getByRole('button', { name: 'Glass project', exact: true }).click();
  await sidebar.getByRole('button', { name: 'Edit', exact: true }).last().click();
  await assertCentered();
  await page.keyboard.press('Escape');
  await expect(editor).toBeHidden();
  await sidebar.getByRole('button', { name: 'Glass project', exact: true }).click();
  await sidebar.getByTestId('project-workspace-open').click();
  await assertCentered();
  await page.keyboard.press('Escape');
  await expect(editor).toBeHidden();
  await sidebar.getByRole('button', { name: 'Glass project', exact: true }).click();
  await sidebar.getByRole('button', { name: 'Project memory', exact: true }).click();
  await assertCentered();
  await page.keyboard.press('Escape');
  await expect(editor).toBeHidden();
  await sidebar.getByRole('button', { name: 'Glass project', exact: true }).click();
  await sidebar.getByRole('button', { name: 'Delete Project', exact: true }).last().click();
  await assertCentered();
  await editor.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(editor).toBeHidden();
});

test('creating a project opens its own new conversation and first send uses that project', async ({ page }, testInfo) => {
  await page.goto('/chat/conv-active');
  const sidebar = page.getByTestId('chat-history-sidebar');
  await page.getByRole('textbox', { name: 'Type a message...' }).fill('Draft for the legacy conversation');
  await sidebar.getByRole('button', { name: 'Legacy project', exact: true }).click();
  await sidebar.getByRole('button', { name: 'New Project', exact: true }).click();
  await page.getByPlaceholder('Enter project name...').fill('Fresh project');
  await page.getByLabel('Primary folder', { exact: true }).fill('D:\\work\\fresh-project');
  const editor = page.getByRole('dialog');
  await expect(editor).toBeVisible();
  const bounds = await editor.boundingBox();
  expect(bounds!.width).toBeGreaterThan(550);
  await page.screenshot({ path: testInfo.outputPath('new-project-editor.png') });
  await page.setViewportSize({ width: 760, height: 700 });
  await expect(page.getByTestId('project-save')).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('new-project-editor-narrow.png') });
  await page.getByTestId('project-save').click();
  await expect(page).toHaveURL(/\/chat$/);
  await expect(page.getByTestId('project-new-conversation')).toContainText('Fresh project');
  expect(await page.evaluate(() => (window as any).__PROJECT_CREATE_INPUT__.workspaceRoots)).toEqual(['D:\\work\\fresh-project']);
  await expect(page.getByRole('textbox', { name: 'Type a message...' })).toHaveCount(0);
  await expect(page.getByTestId('chat-input-toolbar')).toHaveCount(0);
  await expect(sidebar.getByTestId('conversation-item-conv-active')).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__CREATE_CONVERSATION_ARGS__.length)).toBe(0);
  await page.screenshot({ path: testInfo.outputPath('new-project-conversation.png') });
  await page.setViewportSize({ width: 680, height: 820 });
  await expect.poll(() => page.getByTestId('chat-workspace-surface').evaluate(element => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(1);
  await page.screenshot({ path: testInfo.outputPath('new-project-conversation-narrow.png') });
  await page.getByTestId('project-start-chat').click();
  await expect(page.getByTestId('project-new-conversation')).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Type a message...' })).toHaveValue('');
  await page.getByRole('textbox', { name: 'Type a message...' }).fill('First task in the fresh project');
  await page.getByRole('textbox', { name: 'Type a message...' }).press('Enter');
  await expect.poll(() => page.evaluate(() => (window as any).__CREATE_CONVERSATION_ARGS__[0]?.projectId)).toBe('project-1');
  await expect(page).toHaveURL(/\/chat\/conv-new$/);
});

test('new chat cannot accept input under the previous conversation draft owner', async ({ page }) => {
  await page.goto('/chat/conv-active');
  await expect(page.getByTestId('chat-input-textarea')).toBeEnabled();
  const owner = await page.getByTestId('chat-history-sidebar').getByRole('button', { name: 'New Chat', exact: true })
    .evaluate(button => {
      button.click();
      const input = document.querySelector<HTMLTextAreaElement>('[data-testid="chat-input-textarea"]');
      return { key: input?.dataset.draftKey, canType: input != null && !input.disabled };
    });
  expect(owner.canType && owner.key !== '__new__:project-legacy').toBe(false);
});

test('project drafts remain separate and browser navigation restores the selected project', async ({ page }) => {
  await page.goto('/chat/conv-active');
  const sidebar = page.getByTestId('chat-history-sidebar');
  const input = page.getByTestId('chat-input-textarea');
  await sidebar.getByRole('button', { name: 'New Chat', exact: true }).click();
  await input.fill('Unsent legacy project draft');
  await sidebar.getByRole('button', { name: 'Legacy project', exact: true }).click();
  await sidebar.getByRole('button', { name: 'New Project', exact: true }).click();
  await page.getByPlaceholder('Enter project name...').fill('Fresh project');
  await page.getByTestId('project-save').click();
  await page.getByTestId('project-start-chat').click();
  await expect(input).toHaveValue('');
  await input.fill('Unsent fresh project draft');
  await sidebar.getByRole('button', { name: 'Fresh project', exact: true }).click();
  await sidebar.getByRole('button', { name: 'Legacy project', exact: true }).click();
  await expect(page.getByTestId('project-new-conversation')).toContainText('Legacy project');
  await expect(input).toHaveCount(0);
  await page.getByTestId('project-start-chat').click();
  await expect(input).toHaveValue('Unsent legacy project draft');
  await page.goBack();
  await expect(sidebar.getByRole('button', { name: 'Fresh project', exact: true })).toBeVisible();
  await expect(input).toHaveValue('Unsent fresh project draft');
  await page.goForward();
  await expect(sidebar.getByRole('button', { name: 'Legacy project', exact: true })).toBeVisible();
  await expect(input).toHaveValue('Unsent legacy project draft');
  expect(await page.evaluate(() => (window as any).__CREATE_CONVERSATION_ARGS__.length)).toBe(0);
});

test('new chat stays an unpersisted draft until the first send', async ({ page }) => {
  await page.goto('/chat/conv-active');
  await page.getByTestId('chat-history-sidebar').getByRole('button', { name: 'New Chat' }).click();
  await expect(page).toHaveURL(/\/chat$/);
  await expect.poll(() => page.evaluate(() =>
    (window as unknown as { __CREATE_CONVERSATION_ARGS__: Array<Record<string, unknown>> })
      .__CREATE_CONVERSATION_ARGS__.length,
  )).toBe(0);
  await expect(page.getByTestId('conversation-item-conv-new')).toHaveCount(0);

  await page.reload();
  await expect(page.getByTestId('conversation-item-conv-new')).toHaveCount(0);
  expect(await page.evaluate(() =>
    (window as unknown as { __CREATE_CONVERSATION_ARGS__: Array<Record<string, unknown>> })
      .__CREATE_CONVERSATION_ARGS__.length,
  )).toBe(0);
});

test('new chat resets the previous conversation persona before first persistence', async ({ page }) => {
  await page.goto('/chat/conv-active');
  await expect(page.getByRole('button', { name: 'Personas' })).toHaveAttribute('title', /Programmer/);

  await page.getByTestId('chat-history-sidebar').getByRole('button', { name: 'New Chat' }).click();
  await expect(page).toHaveURL(/\/chat$/);
  await expect(page.getByRole('button', { name: 'Personas' })).toHaveAttribute('title', /Default/);
  await page.getByPlaceholder('Type a message...').fill('Hello from a fresh draft.');
  await page.getByPlaceholder('Type a message...').press('Enter');

  await expect.poll(() => page.evaluate(() =>
    (window as unknown as { __CREATE_CONVERSATION_ARGS__: Array<Record<string, unknown>> })
      .__CREATE_CONVERSATION_ARGS__[0],
  )).toMatchObject({ personaId: 'default' });
});

test('failed first persistence keeps the local draft available for retry', async ({ page }) => {
  await page.goto('/chat/conv-active');
  await page.getByTestId('chat-history-sidebar').getByRole('button', { name: 'New Chat' }).click();
  await page.evaluate(() => localStorage.setItem('e2e-fail-next-conversation-create', '1'));
  const input = page.getByPlaceholder('Type a message...');
  await input.fill('Keep this draft when persistence fails.');

  await input.press('Enter');

  await expect.poll(() => page.evaluate(() =>
    (window as unknown as { __CREATE_CONVERSATION_ARGS__: Array<Record<string, unknown>> })
      .__CREATE_CONVERSATION_ARGS__.length,
  )).toBe(1);
  await expect(input).toHaveValue('Keep this draft when persistence fails.');
  await expect(page).toHaveURL(/\/chat$/);
  await expect(page.getByTestId('conversation-item-conv-new')).toHaveCount(0);
});

test('rejected first turn rolls back its empty conversation and keeps the draft', async ({ page }) => {
  await page.goto('/chat/conv-active');
  await page.getByTestId('chat-history-sidebar').getByRole('button', { name: 'New Chat' }).click();
  await page.evaluate(() => localStorage.setItem('e2e-fail-next-agent-launch', '1'));
  const input = page.getByPlaceholder('Type a message...');
  await input.fill('Keep this draft when the agent launch is rejected.');

  await input.press('Enter');

  await expect.poll(() => page.evaluate(() =>
    (window as unknown as { __CREATE_CONVERSATION_ARGS__: Array<Record<string, unknown>> })
      .__CREATE_CONVERSATION_ARGS__.length,
  )).toBe(1);
  await expect.poll(() => page.evaluate(() =>
    (window as unknown as { __ARCHIVE_COMMANDS__: string[] })
      .__ARCHIVE_COMMANDS__.filter((command) => command === 'delete_conversation_cmd').length,
  )).toBe(1);
  await expect(input).toHaveValue('Keep this draft when the agent launch is rejected.');
  await expect(page).toHaveURL(/\/chat$/);
  await expect(page.getByTestId('conversation-item-conv-new')).toHaveCount(0);
});

test('a deferred first send never redirects after the user selects another conversation', async ({ page }) => {
  await page.goto('/chat/conv-active');
  const sidebar = page.getByTestId('chat-history-sidebar');
  await sidebar.getByRole('button', { name: 'New Chat' }).click();
  await page.evaluate(() => localStorage.setItem('e2e-delay-next-agent-launch', '1'));
  const input = page.getByPlaceholder('Type a message...');
  await input.fill('Start this conversation without stealing later navigation.');

  await input.press('Enter');
  await sidebar.getByTestId('conversation-item-conv-active').click();
  await expect(page).toHaveURL(/\/chat\/conv-active$/);

  await expect.poll(() => page.evaluate(() =>
    (window as unknown as { __CREATE_CONVERSATION_ARGS__: Array<Record<string, unknown>> })
      .__CREATE_CONVERSATION_ARGS__.length,
  )).toBe(1);
  await page.waitForTimeout(400);
  await expect(page).toHaveURL(/\/chat\/conv-active$/);
  await expect(sidebar.getByTestId('conversation-item-conv-new')).toHaveCount(1);
});

test('first persistence is single-flight when Enter is pressed repeatedly', async ({ page }) => {
  await page.goto('/chat/conv-active');
  await page.getByTestId('chat-history-sidebar').getByRole('button', { name: 'New Chat' }).click();
  await page.evaluate(() => localStorage.setItem('e2e-delay-conversation-create', '1'));
  const input = page.getByPlaceholder('Type a message...');
  await input.fill('Create exactly one conversation.');

  await input.press('Enter');
  await input.press('Enter');
  await page.waitForTimeout(500);

  expect(await page.evaluate(() =>
    (window as unknown as { __CREATE_CONVERSATION_ARGS__: Array<Record<string, unknown>> })
      .__CREATE_CONVERSATION_ARGS__.length,
  )).toBe(1);
});

test('archived conversations can be restored from the sidebar manager', async ({ page }) => {
  await page.goto('/chat/conv-active');

  await expect(page.getByTestId('chat-archive-nav')).toBeVisible();
  await page.getByTestId('chat-archive-nav').click();

  const archivedItem = page.getByTestId('archived-conversation-conv-archived');
  await expect(archivedItem).toContainText('Archived conversation');
  await expect(archivedItem.getByRole('button', { name: 'Delete' })).toBeVisible();
  await archivedItem.click();
  await expect(page.getByTestId('archived-conversation-banner')).toContainText('read-only');
  await expect(page.getByText('Keep this archived question available.')).toBeVisible();
  await expect(page.getByText('This archived answer remains readable.')).toBeVisible();
  await expect(page.getByPlaceholder('Type a message...')).toBeHidden();

  await archivedItem.getByRole('button', { name: 'Unarchive' }).click();
  await expect(archivedItem).toBeHidden();
  await expect.poll(() => page.evaluate(() =>
    (window as unknown as { __ARCHIVE_COMMANDS__: string[] }).__ARCHIVE_COMMANDS__,
  )).toContain('unarchive_conversation_cmd');

  await expect(page.getByText('Archived conversation', { exact: true })).toBeVisible();
});

test('archived deletion stays responsive and invokes the destructive command once', async ({ page }) => {
  await page.goto('/chat/conv-active');
  await page.getByTestId('chat-archive-nav').click();
  await page.evaluate(() => localStorage.setItem('e2e-delay-conversation-delete', '1'));

  const archivedItem = page.getByTestId('archived-conversation-conv-archived');
  await archivedItem.getByRole('button', { name: 'Delete' }).click();
  await expect(archivedItem).toBeHidden();

  await expect.poll(() => page.evaluate(() =>
    (window as unknown as { __ARCHIVE_COMMANDS__: string[] })
      .__ARCHIVE_COMMANDS__.filter((command) => command === 'delete_conversation_cmd').length,
  ), { timeout: 7_000 }).toBe(1);

  await page.getByRole('button', { name: 'Back to conversations' }).click();
  await expect(page.getByTestId('conversation-item-conv-active')).toBeVisible();
  await page.waitForTimeout(900);
  expect(await page.evaluate(() =>
    (window as unknown as { __ARCHIVE_COMMANDS__: string[] })
      .__ARCHIVE_COMMANDS__.filter((command) => command === 'delete_conversation_cmd').length,
  )).toBe(1);
});

test('a direct archived conversation link opens read-only without joining the active list', async ({ page }) => {
  await page.goto('/chat/conv-archived');

  const banner = page.getByTestId('archived-conversation-banner');
  await expect(banner).toBeVisible();
  await expect(page.getByTestId('archived-conversation-conv-archived')).toHaveAttribute(
    'aria-current',
    'page',
  );
  await expect(page.getByTestId('conversation-item-conv-archived')).toHaveCount(0);
  await expect(page.getByPlaceholder('Type a message...')).toHaveCount(0);

  await page.keyboard.press('Control+Shift+B');
  const dock = page.getByTestId('browser-dock');
  await expect(dock).toBeVisible();
  await expect(dock.getByRole('button', { name: 'Point out' })).toHaveCount(0);
  await expect(dock.getByRole('button', { name: 'Coordinate region' })).toHaveCount(0);
  await expect(dock.getByRole('button', { name: 'Send text' })).toHaveCount(0);

  await banner.getByRole('button', { name: 'Restore' }).click();
  await expect(banner).toBeHidden();
  await expect(page.getByTestId('conversation-item-conv-archived')).toBeVisible();
  await expect(page.getByPlaceholder('Type a message...')).toBeVisible();
  await expect(dock.getByRole('button', { name: 'Point out' })).toBeVisible();
});

test('responsive sidebar collapse is temporary and restores the user preference', async ({ page }) => {
  await page.setViewportSize({ width: 1000, height: 720 });
  await page.goto('/chat/conv-active');

  const sidebar = page.getByTestId('chat-history-sidebar');
  await expect(sidebar).toHaveAttribute('data-collapsed', 'false');

  await page.setViewportSize({ width: 700, height: 720 });
  await expect(sidebar).toHaveAttribute('data-collapsed', 'true');
  await expect.poll(() => page.evaluate(() => localStorage.getItem('chat-sidebar-collapsed')))
    .toBeNull();

  await page.setViewportSize({ width: 1000, height: 720 });
  await expect(sidebar).toHaveAttribute('data-collapsed', 'false');
});

test('typing shortcuts do not toggle the conversation sidebar', async ({ page }) => {
  await page.setViewportSize({ width: 1000, height: 720 });
  await page.goto('/chat/conv-active');

  const sidebar = page.getByTestId('chat-history-sidebar');
  const composer = page.getByPlaceholder('Type a message...');
  await composer.focus();
  await composer.press('Control+b');
  await expect(sidebar).toHaveAttribute('data-collapsed', 'false');
  await expect.poll(() => page.evaluate(() => localStorage.getItem('chat-sidebar-collapsed')))
    .toBeNull();
});


test('project workspace exposes its folders and explicitly saves a new primary root', async ({ page }) => {
  await page.goto('/chat/conv-active');
  const sidebar = page.getByTestId('chat-history-sidebar');
  await sidebar.getByRole('button', { name: 'Legacy project', exact: true }).click();
  await sidebar.getByRole('button', { name: 'New Project', exact: true }).click();
  await page.getByPlaceholder('Enter project name...').fill('Folder project');
  await page.getByLabel('Primary folder', { exact: true }).fill('D:/work/primary');
  await page.getByTestId('project-save').click();
  await sidebar.getByRole('button', { name: 'Folder project', exact: true }).click();
  await page.getByTestId('project-workspace-open').click();
  await expect(page.getByTestId('project-workspace-folders')).toContainText('D:/work/primary');
  await page.getByTestId('project-workspace-folders').click();
  await expect(page.getByLabel('Primary folder', { exact: true })).toHaveValue('D:/work/primary');
  await page.getByRole('button', { name: 'Add folder', exact: true }).click();
  await page.getByLabel('Additional folder', { exact: true }).fill('D:/work/shared');
  await page.getByRole('button', { name: 'Make primary', exact: true }).click();
  await expect(page.getByLabel('Primary folder', { exact: true })).toHaveValue('D:/work/shared');
  await page.getByTestId('project-save').click();
  await expect.poll(() => page.evaluate(() => (window as any).__PROJECT_UPDATE_INPUT__?.workspaceRoots))
    .toEqual(['D:/work/shared', 'D:/work/primary']);
});


test('review preserves drafts, deduplicates selected feedback and keeps old findings stale', async ({ page }, testInfo) => {
  await page.goto('/chat/conv-active'); await page.getByTestId('chat-input-textarea').waitFor();
  await page.evaluate(() => {
    const state=window as unknown as {__TAURI_INTERNALS__:{invoke:(command:string,args?:Record<string,unknown>)=>Promise<unknown>};__reviewChanged:boolean;__reviewActions:string[]};
    const original=state.__TAURI_INTERNALS__.invoke; state.__reviewChanged=false; state.__reviewActions=[];
    let review:any=null;
    state.__TAURI_INTERNALS__.invoke=async(command,args)=>{
      if(command!=='code_review_cmd')return original(command,args);
      const request=args?.request as any; state.__reviewActions.push(request.action);
      if(request.action==='list')return review?[{id:review.id,mode:review.mode,baseRef:review.baseRef,createdAt:review.createdAt}]:[];
      if(request.action==='start')review={id:'review-one',mode:request.mode,baseRef:'HEAD',baseSha:'a'.repeat(40),workspaceRoot:'D:/work/project',createdAt:'2026-10-03T03:00:00Z',snapshot:{revision:'a'.repeat(64),headSha:'b'.repeat(40),baseline:'a'.repeat(40),truncated:false,files:[{path:'代码.txt',patch:'--- a/代码.txt\n+++ b/代码.txt\n@@ -1,2 +1,2 @@\n first\n-old\n+new\n',truncated:false,binary:false}]},findings:[],pullRequest:null};
      if(request.action==='add')review.findings=[{id:'finding-one',...request.finding,status:'open',stale:false}];
      if(request.action==='feedback')return {marker:'[nexa-review:packet-one]',text:'[nexa-review:packet-one]\nFix the selected current finding.'};
      if(request.action==='get'&&review&&state.__reviewChanged){review.snapshot.revision='c'.repeat(64);review.findings=review.findings.map((finding:any)=>({...finding,stale: finding.status!=='resolved'}));}
      if(request.action==='disposition')review.findings=review.findings.map((finding:any)=>({...finding,status:request.status,stale:false}));
      if(request.action==='attach_pr')review.pullRequest={url:request.url,title:'Correct the parser',headSha:'d'.repeat(40),state:'OPEN',draft:false,reviewDecision:null,observedAt:'2026-10-03T03:01:00Z',matchesLocalHead:false,partial:true,checks:[{name:'Rust tests',state:'SUCCESS',url:null}],unresolvedThreads:[{id:'thread-one',path:'代码.txt',line:2,outdated:true,body:'Please check the changed branch.',url:null}]};
      return structuredClone(review);
    };
  });
  const input=page.getByTestId('chat-input-textarea'); await input.fill('Keep this draft');
  const open=async()=>{await page.keyboard.press('Control+Shift+P');const palette=page.getByRole('dialog',{name:/command palette/i});await palette.getByRole('combobox').fill('Code review');await palette.getByRole('option',{name:'Code review',exact:true}).click();};
  await open(); const panel=page.getByTestId('code-review-panel');
  await panel.getByRole('button',{name:'Start review',exact:true}).click(); await panel.getByRole('button',{name:'new 2',exact:true}).click();
  await panel.getByRole('textbox',{name:'Finding title',exact:true}).fill('Wrong result');
  await panel.getByRole('textbox',{name:'Explain the bug and its impact',exact:true}).fill('The changed branch drops the result.');
  await panel.getByRole('button',{name:'Record finding',exact:true}).click();
  await panel.getByRole('checkbox',{name:'Wrong result',exact:true}).check();
  await panel.getByRole('button',{name:/Add selected fixes to draft/}).click();
  await expect(input).toHaveValue('Keep this draft\n\n[nexa-review:packet-one]\nFix the selected current finding.');
  await open(); await panel.getByRole('checkbox',{name:'Wrong result',exact:true}).check();await panel.getByRole('button',{name:/Add selected fixes to draft/}).click();
  expect((await input.inputValue()).match(/nexa-review:packet-one/g)).toHaveLength(1);
  await open(); await page.evaluate(()=>{(window as unknown as {__reviewChanged:boolean}).__reviewChanged=true;});
  await panel.getByRole('button',{name:'Refresh diff',exact:true}).click(); await expect(panel.getByRole('checkbox',{name:'Wrong result',exact:true})).toBeDisabled();
  await expect(panel.getByRole('button',{name:'Accept',exact:true})).toBeDisabled();
  await panel.getByRole('textbox',{name:'GitHub PR URL',exact:true}).fill('https://github.com/example/repo/pull/1');
  await panel.getByRole('button',{name:'Read / refresh PR',exact:true}).click(); await expect(panel).toContainText('The PR head differs from this local review.');
  await panel.getByText('Checks at this SHA (1)',{exact:true}).click(); await expect(panel).toContainText('Rust tests');
  await page.screenshot({path:testInfo.outputPath('code-review.png')});
  await panel.getByRole('button',{name:'Verified fixed',exact:true}).click(); await expect(panel).toContainText('Resolved');
  await page.keyboard.press('Escape'); await expect(input).toHaveValue(/^Keep this draft/);
  await input.fill('/review');await page.keyboard.press('Enter');await expect(panel).toBeVisible();
  expect(await page.evaluate(()=>(window as unknown as {__reviewActions:string[]}).__reviewActions)).not.toContain('publish');
});
