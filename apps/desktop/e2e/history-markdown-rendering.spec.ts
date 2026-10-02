import { expect, test, type Page } from '@playwright/test';
import ts from 'typescript';

type RenderSnapshot = {
  messageBubble: number;
  codeBlock: number;
  highlight: number;
  byBlock: Record<string, { codeBlock: number; highlight: number }>;
};

/** Instrument the served production module, without replacing components,
 * adding a React wrapper, or changing a memo boundary. The Highlight counter
 * runs in the render-prop callback invoked by the actual Prism component. */
function instrumentModule(source: string, kind: 'message' | 'markdown'): string {
  const file = ts.createSourceFile('served-module.js', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  const functions: Array<ts.FunctionDeclaration | ts.FunctionExpression> = [];
  const name = kind === 'message' ? /^MessageBubbleInner\d*$/ : /^CodeBlock\d*$/;
  const visit = (node: ts.Node) => {
    if ((ts.isFunctionDeclaration(node) || ts.isFunctionExpression(node)) && node.name && name.test(node.name.text)) functions.push(node);
    ts.forEachChild(node, visit);
  };
  visit(file);
  expect(functions.length, `${kind}: expected one production render function in served JavaScript`).toBe(1);
  const target = functions[0];
  if (!target.body) throw new Error(`${kind}: render function has no body`);
  const argument = target.parameters[0]?.name;
  if (!argument || !ts.isObjectBindingPattern(argument)) throw new Error(`${kind}: expected destructured component props`);
  const property = kind === 'message' ? 'msg' : 'code';
  const binding = argument.elements.find(element => (element.propertyName ?? element.name).getText(file) === property);
  if (!binding || !ts.isIdentifier(binding.name)) throw new Error(`${kind}: missing ${property} prop binding`);
  const value = binding.name.text;
  const patches = [{
    position: target.body.getStart(file) + 1,
    text: `\nglobalThis.__HISTORY_MARKDOWN_AUDIT__?.${kind === 'message' ? 'messageBubble' : 'codeBlock'}(${value}${kind === 'message' ? '.id' : ''});\n`,
  }];
  if (kind === 'markdown') {
    const callbacks: ts.ArrowFunction[] = [];
    const findHighlight = (node: ts.Node) => {
      if (ts.isArrowFunction(node)) {
        const parameter = node.parameters[0]?.name;
        if (parameter && ts.isObjectBindingPattern(parameter)) {
          const properties = parameter.elements.map(element => (element.propertyName ?? element.name).getText(file));
          if (['tokens', 'getLineProps', 'getTokenProps'].every(field => properties.includes(field))) callbacks.push(node);
        }
      }
      ts.forEachChild(node, findHighlight);
    };
    findHighlight(target.body);
    expect(callbacks.length, 'expected one real Prism Highlight render-prop callback').toBe(1);
    const body = callbacks[0].body;
    const count = `globalThis.__HISTORY_MARKDOWN_AUDIT__?.highlight(${value})`;
    if (ts.isBlock(body)) {
      patches.push({ position: body.getStart(file) + 1, text: `\n${count};\n` });
    } else {
      patches.push({ position: body.getStart(file), text: `(${count}, ` });
      patches.push({ position: body.end, text: ')' });
    }
  }
  return patches.sort((left, right) => right.position - left.position)
    .reduce((body, patch) => body.slice(0, patch.position) + patch.text + body.slice(patch.position), source);
}

async function crossFrames(page: Page): Promise<number> {
  return page.evaluate(() => new Promise<number>(resolve => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

test('live tool progress does not render or re-highlight twenty historical rich code blocks', async ({ page }, testInfo) => {
  const codeBlocks = Array.from({ length: 20 }, (_, index) => [
    `const HISTORY_RICH_BLOCK_${String(index).padStart(2, '0')} = [1, 2, 3];`,
    `function total${index}(values) {`,
    '  return values.map(value => value * 2).reduce((sum, value) => sum + value, 0);',
    '}',
  ].join('\n'));
  const markdown = '# Historical rich code examples\n\n' + codeBlocks.map((code, index) => `### Example ${index + 1}\n\n\`\`\`javascript\n${code}\n\`\`\``).join('\n\n');
  expect(markdown.length).toBeLessThan(16 * 1024);
  const instrumented = new Set<string>();
  for (const [kind, pattern] of [
    ['message', /\/src\/components\/chat\/MessageBubble\.tsx(?:\?.*)?$/],
    ['markdown', /\/src\/components\/chat\/markdownComponents\.tsx(?:\?.*)?$/],
  ] as const) {
    await page.route(pattern, async route => {
      const response = await route.fetch();
      const body = instrumentModule(await response.text(), kind);
      const headers = response.headers();
      delete headers['content-encoding'];
      delete headers['content-length'];
      instrumented.add(kind);
      await route.fulfill({ status: response.status(), headers, body });
    });
  }
  await page.addInitScript(({ markdown }) => {
    localStorage.setItem('nexa-locale', 'en');
    localStorage.removeItem('last-insights-at');
    const now = '2026-10-02T00:00:00Z';
    const conversationId = 'rich-history';
    const conversation = { id: conversationId, title: 'Rich history isolation', provider: 'open_ai', model: 'gpt-4.1', systemPrompt: '', projectId: null, personaId: null, initialAutoTitlePending: false, createdAt: now, updatedAt: now };
    const config = { id: 'rich-config', name: 'Test model', provider: 'open_ai', apiKey: '', baseUrl: null, model: 'gpt-4.1', temperature: 0.3, maxTokens: 4096, contextWindow: 128000, isDefault: true, createdAt: now, updatedAt: now };
    const message = (id: string, role: string, content: string, sortOrder: number) => ({ id, conversationId, role, content, sortOrder, toolCallId: null, toolCalls: [], artifacts: null, thinking: null, tokenCount: 1, createdAt: now, imageAttachments: null });
    const messages = [
      message('rich-history-user', 'user', 'Show the historical code examples.', 0),
      message('rich-history-answer', 'assistant', markdown, 1),
      message('rich-active-user', 'user', 'Continue the current file inspection.', 2),
    ];
    const turn = { id: 'rich-history-turn', conversationId, userMessageId: 'rich-history-user', assistantMessageId: 'rich-history-answer', status: 'success', trace: null, createdAt: now, updatedAt: now, finishedAt: now };
    const oldest = { sortOrder: 0, messageId: 'rich-history-user' };
    const newest = { sortOrder: 2, messageId: 'rich-active-user' };
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, { event: string; handler: number }>();
    const pendingInsights = new Map<string, (value: unknown[]) => void>();
    const insightEvents: Array<{ command: string; phase: 'requested' | 'released'; at: number }> = [];
    let sequence = 0;
    let eventSequence = 0;
    const counts: RenderSnapshot = { messageBubble: 0, codeBlock: 0, highlight: 0, byBlock: {} };
    const countCode = (kind: 'codeBlock' | 'highlight', code: string) => {
      const id = /HISTORY_RICH_BLOCK_(\d{2})/.exec(code)?.[1];
      if (!id) return;
      counts[kind] += 1;
      (counts.byBlock[id] ??= { codeBlock: 0, highlight: 0 })[kind] += 1;
    };
    const emitProgress = (step: number) => {
      const run = {
        callId: 'rich-live-tool', toolName: 'read_file', status: 'running',
        owner: { id: 'file-workspace', name: 'Workspace', capability: 'File access', description: 'Fixture file access' },
        providerExecuted: false, arguments: '{"path":"tail-progress.txt"}', renderKind: 'generic',
        capabilities: { inputStreaming: 'none', renderKind: 'generic', readOnly: true, destructive: false, concurrencySafe: true, interruptBehavior: 'cancel', resourceKeys: [] },
        progressNote: `Progress ${String(step).padStart(2, '0')}`,
      };
      const payload = { conversationId, runEvent: {
        version: 2, runId: 'rich-live-run', turnId: 'rich-live-turn', eventSeq: ++eventSequence,
        kind: step === 0 ? 'toolStarted' : 'toolProgress', phase: 'tooling', status: 'running',
        visibility: 'user', persistence: 'ephemeral', displayKind: 'tool', importance: 'normal',
        label: 'Read current file', payload: { run }, createdAt: now,
      } };
      let delivered = 0;
      for (const [id, listener] of listeners) {
        if (listener.event === 'agent://run-event') {
          callbacks.get(listener.handler)?.({ event: listener.event, id, payload });
          delivered += 1;
        }
      }
      if (delivered === 0) throw new Error('The real StreamProvider listener is not attached');
    };
    const invoke = async (command: string, args: Record<string, unknown> = {}) => {
      switch (command) {
        case 'plugin:event|listen': { const id = ++sequence; listeners.set(id, { event: String(args.event), handler: Number(args.handler) }); return id; }
        case 'plugin:event|unlisten': listeners.delete(Number(args.eventId)); return null;
        case 'get_wizard_state_cmd': return { completed: true, language: 'en' };
        case 'get_knowledge_gaps_cmd': case 'suggest_explorations_cmd':
          insightEvents.push({ command, phase: 'requested', at: performance.now() });
          return new Promise<unknown[]>(resolve => pendingInsights.set(command, resolve));
        case 'list_agent_configs_cmd': return [config];
        case 'get_model_context_window': return 128000;
        case 'list_conversations_cmd': return [conversation];
        case 'get_conversation_timeline_page_cmd': return {
          conversation, messages, turns: [turn], taskRuns: [],
          entries: [{ anchor: oldest, turnId: turn.id, hasDetails: false, detailRevision: '1' }, { anchor: newest, turnId: null, hasDetails: false, detailRevision: '1' }],
          range: { from: oldest, before: null }, oldestCursor: oldest, newestCursor: newest, hasMoreBefore: false, hasMoreAfter: false,
        };
        case 'get_conversation_cmd': return [conversation, messages];
        case 'get_conversation_turns_cmd': return [turn];
        case 'get_index_stats': return { totalDocuments: 0, totalChunks: 0, ftsRows: 0 };
        case 'get_privacy_config': return { enabled: false, excludePatterns: [], redactPatterns: [] };
        case 'get_agent_task_runs_cmd': case 'list_pending_tool_approvals_cmd': case 'list_interaction_requests_cmd':
        case 'list_sources': case 'list_projects_cmd': case 'list_personas_cmd': case 'list_checkpoints_cmd':
        case 'list_skills_cmd': case 'list_mcp_servers_cmd': case 'list_user_memories_cmd': case 'get_conversation_sources_cmd':
        case 'get_conversation_file_changes_cmd': return [];
        default: return null;
      }
    };
    Object.assign(window, {
      __HISTORY_MARKDOWN_AUDIT__: {
        messageBubble: (id: string) => { if (id === 'rich-history-answer') counts.messageBubble += 1; },
        codeBlock: (code: string) => countCode('codeBlock', code),
        highlight: (code: string) => countCode('highlight', code),
        snapshot: () => JSON.parse(JSON.stringify(counts)),
        pendingInsights: () => [...pendingInsights.keys()].sort(),
        insightEvents,
        releaseInsights: () => {
          if (pendingInsights.size !== 2) throw new Error('Both actual startup insight requests must be pending');
          for (const [command, resolve] of pendingInsights) {
            insightEvents.push({ command, phase: 'released', at: performance.now() });
            resolve([]);
          }
          pendingInsights.clear();
        },
        emitProgress,
      },
      __TAURI_INTERNALS__: {
        invoke, metadata: { currentWindow: { label: 'main' } }, convertFileSrc: (path: string) => path,
        transformCallback: (callback: (event: unknown) => void) => { const id = ++sequence; callbacks.set(id, callback); return id; },
        unregisterCallback: (id: number) => callbacks.delete(id),
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: (_event: string, id: number) => listeners.delete(id) },
    });
  }, { markdown });

  const pageErrors: string[] = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.goto('/chat/rich-history');
  const history = page.locator('[data-timeline-key="turn-rich-history-turn"]');
  await expect(history.locator('pre code')).toHaveCount(20);
  await expect(history.locator('pre[data-code-presentation="plain"]')).toHaveCount(0);
  expect(await history.locator('pre code .token').count()).toBeGreaterThan(20);
  expect([...instrumented].sort()).toEqual(['markdown', 'message']);
  await expect.poll(() => page.evaluate(() => (window as any).__HISTORY_MARKDOWN_AUDIT__.pendingInsights())).toEqual(['get_knowledge_gaps_cmd', 'suggest_explorations_cmd']);

  await page.evaluate(async () => {
    const { streamStore } = await import('/src/lib/streamStore.ts');
    streamStore.startStream('rich-history');
    streamStore.bindTurnHandle('rich-history', { conversationId: 'rich-history', runId: 'rich-live-run', turnId: 'rich-live-turn', state: 'running' });
    (window as any).__HISTORY_MARKDOWN_AUDIT__.emitProgress(0);
  });
  const liveTool = page.getByTestId('tool-call-card');
  await expect(liveTool).toContainText('Progress 00');
  await crossFrames(page);
  const snapshot = () => page.evaluate<RenderSnapshot>(() => (window as any).__HISTORY_MARKDOWN_AUDIT__.snapshot());
  const before = await snapshot();
  expect(before.messageBubble).toBeGreaterThan(0);
  expect(Object.keys(before.byBlock)).toHaveLength(20);
  expect(Object.values(before.byBlock).every(block => block.codeBlock > 0 && block.highlight > 0)).toBe(true);

  // Complete a real AppShell background update after the baseline. An unchanged
  // locale must not republish a new context and re-highlight historical code.
  await page.evaluate(() => (window as any).__HISTORY_MARKDOWN_AUDIT__.releaseInsights());
  await expect.poll(() => page.evaluate(() => localStorage.getItem('last-insights-at'))).not.toBeNull();
  await crossFrames(page);
  const afterInsights = await snapshot();
  const frames: number[] = [];
  for (let step = 1; step <= 12; step += 1) {
    await page.evaluate(step => (window as any).__HISTORY_MARKDOWN_AUDIT__.emitProgress(step), step);
    frames.push(await crossFrames(page));
    await expect(liveTool).toContainText(`Progress ${String(step).padStart(2, '0')}`);
  }
  const after = await snapshot();
  expect(frames.every((time, index) => index === 0 || time > frames[index - 1])).toBe(true);
  expect(afterInsights, 'an unrelated AppShell update must not change the locale context').toEqual(before);
  expect(after).toEqual(before);
  await expect(history.locator('pre code')).toHaveCount(20);
  await expect(history.locator('pre[data-code-presentation="plain"]')).toHaveCount(0);
  expect(pageErrors).toEqual([]);
  await testInfo.attach('historical-rich-markdown-render-counts', {
    contentType: 'application/json',
    body: Buffer.from(JSON.stringify({ scenario: 'actual ChatPage + StreamProvider; one AppShell background update; 20 Prism blocks; 12 cross-frame toolProgress events', before, afterInsights, after, frames, insightEvents: await page.evaluate(() => (window as any).__HISTORY_MARKDOWN_AUDIT__.insightEvents) }, null, 2)),
  });

  // Real locale changes still update both the provider value and loaded text.
  await page.getByRole('link', { name: 'Settings', exact: true }).click();
  const language = page.locator('#settings-language');
  await expect(language).toBeVisible();
  if (await language.evaluate(element => element.tagName === 'SELECT')) await language.selectOption('zh-CN');
  else {
    await language.click();
    await page.getByRole('option', { name: '简体中文', exact: true }).click();
  }
  await expect(page.locator('html')).toHaveAttribute('lang', 'zh-CN');
  await expect(page.getByRole('link', { name: '设置', exact: true })).toBeVisible();
  await expect(page.getByText('选择显示语言', { exact: true })).toBeVisible();
  if (await language.evaluate(element => element.tagName === 'SELECT')) await language.selectOption('en');
  else {
    await language.click();
    await page.getByRole('option', { name: 'English', exact: true }).click();
  }
  await expect(page.locator('html')).toHaveAttribute('lang', 'en');
  await expect(page.getByRole('link', { name: 'Settings', exact: true })).toBeVisible();
  await expect(page.getByText('Choose your preferred display language', { exact: true })).toBeVisible();
});
