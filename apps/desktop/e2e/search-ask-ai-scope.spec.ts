import { expect, test } from './timeline-test';
import { RUN_EVENT_FIXTURE_INIT_SCRIPT } from './run-event-fixture';

test.beforeEach(async ({ page }) => {
  await page.addInitScript({ content: RUN_EVENT_FIXTURE_INIT_SCRIPT });
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale', 'en');

    type Conversation = {
      id: string;
      title: string;
      provider: string;
      model: string;
      systemPrompt: string;
      collectionContext?: null;
      createdAt: string;
      updatedAt: string;
    };

    type Source = {
      id: string;
      kind: string;
      rootPath: string;
      includeGlobs: string[];
      excludeGlobs: string[];
      watchEnabled: boolean;
      createdAt: string;
      updatedAt: string;
    };

    type Message = {
      id: string;
      conversationId: string;
      role: 'system' | 'user' | 'assistant' | 'tool';
      content: string;
      toolCallId: string | null;
      toolCalls: Array<{ id: string; name: string; arguments: string }>;
      artifacts: Record<string, unknown> | null;
      tokenCount: number;
      createdAt: string;
      sortOrder: number;
      thinking: string | null;
      imageAttachments: null;
    };

    const nowIso = new Date().toISOString();
    const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;
    let seq = 0;
    const nextId = (prefix: string) => `${prefix}-${Date.now()}-${seq++}`;
    let callbackSeq = 1;
    let listenerSeq = 1;
    const callbackMap = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, { event: string; handlerId: number }>();

    const emitEvent = (eventName: string, payload: Record<string, unknown>) => {
      const convert = (window as unknown as {
        __toRunEventFixture?: (
          name: string,
          value: Record<string, unknown>,
        ) => { eventName: string; payload: Record<string, unknown> };
      }).__toRunEventFixture;
      const converted = convert?.(eventName, payload);
      if (converted) {
        eventName = converted.eventName;
        payload = converted.payload;
      }
      for (const [listenerId, listener] of listeners.entries()) {
        if (listener.event !== eventName) continue;
        const callback = callbackMap.get(listener.handlerId);
        if (callback) {
          callback({ event: eventName, id: listenerId, payload });
        }
      }
    };

    const sources: Source[] = [
      {
        id: 'source-retries',
        kind: 'local_folder',
        rootPath: 'D:/notes/retries',
        includeGlobs: ['**/*.md'],
        excludeGlobs: [],
        watchEnabled: true,
        createdAt: nowIso,
        updatedAt: nowIso,
      },
      {
        id: 'source-random',
        kind: 'local_folder',
        rootPath: 'D:/notes/random',
        includeGlobs: ['**/*.md'],
        excludeGlobs: [],
        watchEnabled: true,
        createdAt: nowIso,
        updatedAt: nowIso,
      },
    ];

    const conversations: Record<string, Conversation> = {};
    const messagesByConversation: Record<string, Message[]> = {};
    const conversationSources: Record<string, string[]> = {};

    const defaultAgentConfig = {
      id: 'cfg-search-scope',
      name: 'Search Scope Config',
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
        case 'list_agent_configs_cmd':
          return [clone(defaultAgentConfig)];
        case 'get_model_context_window':
          return 1047576;
        case 'list_conversations_cmd':
          return Object.values(conversations).map(clone);
        case 'create_conversation_cmd': {
          const id = 'conv-search-scope';
          const conversation: Conversation = {
            id,
            title: '',
            provider: String(args.provider ?? 'open_ai'),
            model: String(args.model ?? 'gpt-4.1'),
            systemPrompt: String(args.systemPrompt ?? ''),
            collectionContext: null,
            createdAt: new Date().toISOString(),
            updatedAt: new Date().toISOString(),
          };
          conversations[id] = conversation;
          messagesByConversation[id] = [];
          return clone(conversation);
        }
        case 'get_conversation_cmd': {
          const id = String(args.id ?? '');
          return [clone(conversations[id]), clone(messagesByConversation[id] ?? [])];
        }
        case 'get_conversation_turns_cmd':
          return [];
        case 'list_sources':
          return clone(sources);
        case 'get_conversation_sources_cmd':
          return conversationSources[String(args.conversationId ?? '')] ?? [];
        case 'set_conversation_sources_cmd':
          conversationSources[String(args.conversationId ?? '')] = Array.isArray(args.sourceIds)
            ? (args.sourceIds as unknown[]).filter((value): value is string => typeof value === 'string')
            : [];
          return null;
        case 'update_conversation_system_prompt_cmd':
          return null;
        case 'update_conversation_collection_context_cmd':
          return null;
        case 'list_checkpoints_cmd':
          return [];
        case 'compact_conversation_cmd':
          return null;
        case 'agent_stop_cmd':
          return null;
        case 'save_agent_config_cmd':
          return clone(defaultAgentConfig);
        case 'get_index_stats':
          return { totalDocuments: 2, totalChunks: 8, ftsRows: 8 };
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
        case 'list_user_memories_cmd':
          return [];
        case 'list_skills_cmd':
          return [];
        case 'list_mcp_servers_cmd':
          return [];
          return 0;
        case 'get_recent_queries':
          return [];
        case 'search':
        case 'hybrid_search':
          return {
            hits: [],
            total: 0,
            searchMode: 'fts',
          };
        case 'search_conversations_cmd':
          return [];
        case 'agent_chat_cmd': {
          (window as any).__lastAgentArgs = structuredClone(args);
          const conversationId = String(args.conversationId ?? '');
          const current = messagesByConversation[conversationId] ?? [];
          const hasRetryScope = (conversationSources[conversationId] ?? []).includes('source-retries');
          const assistantContent = hasRetryScope ? 'scope-on' : 'scope-off';

          const userMessage: Message = {
            id: nextId('m-user'),
            conversationId,
            role: 'user',
            content: String(args.message ?? ''),
            toolCallId: null,
            toolCalls: [],
            artifacts: null,
            tokenCount: 0,
            createdAt: new Date().toISOString(),
            sortOrder: current.length,
            thinking: null,
            imageAttachments: null,
          };
          const assistantMessage: Message = {
            id: nextId('m-assistant'),
            conversationId,
            role: 'assistant',
            content: assistantContent,
            toolCallId: null,
            toolCalls: [],
            artifacts: null,
            tokenCount: 0,
            createdAt: new Date().toISOString(),
            sortOrder: current.length + 1,
            thinking: null,
            imageAttachments: null,
          };

          messagesByConversation[conversationId] = [...current, userMessage, assistantMessage];

          queueMicrotask(() => {
            emitEvent('agent://run-event', {
              conversationId,
              type: 'textDelta',
              delta: assistantContent,
            });
          });

          setTimeout(() => {
            emitEvent('agent://run-event', {
              conversationId,
              type: 'done',
              message: assistantMessage,
              usageTotal: {
                promptTokens: 120,
                completionTokens: 24,
                totalTokens: 144,
                thinkingTokens: 0,
              },
              lastPromptTokens: 120,
              finishReason: 'stop',
              cached: false,
            });
          }, 40);

          return null;
        }
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
    (window as any).__emitKnowledgeFixture = emitEvent;

    (window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__: unknown }).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: (_event: string, eventId: number) => {
        listeners.delete(eventId);
      },
    };
  });
});

test('carries active search source filters into chat scope when asking AI', async ({ page }) => {
  await page.goto('/');

  await page.locator('button').filter({ hasText: 'Filters' }).click();
  await page.getByRole('button', { name: 'retries' }).click();
  await page.getByPlaceholder('Search by keyword...').fill('Why did the retry guard fail?');
  await page.locator('button').filter({ hasText: 'Ask AI' }).click();

  await expect(page.getByText('scope-on')).toBeVisible();
});

test('recall mode can hand vague clues into chat with the active source scope', async ({ page }) => {
  await page.goto('/');

  await page.locator('button').filter({ hasText: 'Filters' }).click();
  await page.getByRole('button', { name: 'retries' }).click();
  await page.getByText('Recall with vague clues').waitFor();
  await page.getByRole('button', { name: 'Expand' }).click();
  await page.getByLabel('What do you remember?').fill('Something about retry guards and timeout limits.');
  await page.locator('button').filter({ hasText: 'Recall with AI' }).click();

  await expect(page.getByText('scope-on')).toBeVisible();
});

// Search ownership regressions use controlled request completion, not timing assumptions.
// These fixtures run the real React interface without native or paid calls.
async function installAuditSearch(page: import('@playwright/test').Page, manual = false) {
  await page.evaluate((manual) => {
    const auditWindow = window as any;
    const invoke = auditWindow.__TAURI_INTERNALS__.invoke;
    auditWindow.__kbAuditCalls = [];
    auditWindow.__kbAuditPending = {};
    auditWindow.__TAURI_INTERNALS__.invoke = async (cmd: string, args: any = {}) => {
      if (cmd === 'get_feedback_for_query') return [];
      if (cmd !== 'search' && cmd !== 'hybrid_search') return invoke(cmd, args);
      const query = String(args.queryText);
      auditWindow.__kbAuditCalls.push({ cmd, query, filters: args.filters });
      const scoped = args.filters?.sourceIds?.includes('source-retries');
      const title = `${query} evidence ${scoped ? 'retries' : 'random'}`;
      const result = { query, totalMatches: 1, searchTimeMs: 12, searchMode: 'fts', evidenceCards: [{
        chunkId: '11111111-1111-4111-8111-111111111111', documentId: '22222222-2222-4222-8222-222222222222',
        sourceId: scoped ? 'source-retries' : 'source-random', sourceName: scoped ? 'retries' : 'random',
        documentPath: `D:/notes/${scoped ? 'retries' : 'random'}/report.md`, documentTitle: title,
        content: 'Verified fixture source material with a readable detail that belongs to the selected document. This is synthetic audit data.',
        headingPath: ['Overview'], chunkKind: 'text', chunkIndex: 0, score: 0.16, highlights: [],
        evidenceRef: { sourceId: scoped ? 'source-retries' : 'source-random', documentId: '22222222-2222-4222-8222-222222222222', revision: 'fixture-index-revision', documentHash: 'fixture-file-hash', blockId: '11111111-1111-4111-8111-111111111111', contentHash: 'fixture-block-hash', locator: { kind: 'text', byteStart: 0, byteEnd: 100, lineStart: 1, lineEnd: 2 }, extractionMethod: 'native', status: 'current' },
      }] };
      if (query === 'forced failure') throw new Error('Synthetic search failure');
      if (!manual) return result;
      return new Promise(resolve => { auditWindow.__kbAuditPending[query] = () => resolve(result); });
    };
  }, manual);
}

test('late search responses cannot replace the current query', async ({ page }, testInfo) => {
  await page.goto('/');
  await installAuditSearch(page, true);
  const input = page.getByPlaceholder('Search by keyword...');
  await input.fill('old request');
  await expect.poll(() => page.evaluate(() => (window as any).__kbAuditCalls.length)).toBe(1);
  await input.fill('new request');
  await expect.poll(() => page.evaluate(() => (window as any).__kbAuditCalls.length)).toBe(2);
  await page.evaluate(() => (window as any).__kbAuditPending['new request']());
  await expect(page.getByRole('button', { name: 'new request evidence random', exact: true })).toBeVisible();
  await page.evaluate(() => (window as any).__kbAuditPending['old request']());
  await expect(input).toHaveValue('new request');
  await expect(page.getByRole('button', { name: 'new request evidence random', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'old request evidence random', exact: true })).toHaveCount(0);
  await page.screenshot({ path: '.artifacts/kb-ui-audit-2026-10-03-race.png', fullPage: true });
  await testInfo.attach('search-calls', { body: JSON.stringify(await page.evaluate(() => (window as any).__kbAuditCalls), null, 2), contentType: 'application/json' });
});

test('changing sources refreshes evidence under the new scope', async ({ page }, testInfo) => {
  await page.goto('/');
  await installAuditSearch(page);
  await page.getByPlaceholder('Search by keyword...').fill('scope probe');
  await expect(page.getByRole('button', { name: 'scope probe evidence random', exact: true })).toBeVisible();
  const callsBefore = await page.evaluate(() => (window as any).__kbAuditCalls.length);
  await page.locator('button').filter({ hasText: 'Filters' }).click();
  await page.getByRole('button', { name: 'retries', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__kbAuditCalls.length)).toBeGreaterThan(callsBefore);
  await expect(page.getByRole('button', { name: 'scope probe evidence random', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'scope probe evidence retries', exact: true })).toBeVisible();
  await testInfo.attach('search-calls', { body: JSON.stringify(await page.evaluate(() => (window as any).__kbAuditCalls), null, 2), contentType: 'application/json' });
});

test('card follow-up retains the active search source scope', async ({ page }) => {
  await page.goto('/');
  await installAuditSearch(page);
  await page.locator('button').filter({ hasText: 'Filters' }).click();
  await page.getByRole('button', { name: 'retries', exact: true }).click();
  await page.getByPlaceholder('Search by keyword...').fill('retry guard');
  await expect(page.getByRole('button', { name: 'retry guard evidence retries', exact: true })).toBeVisible();
  const cardAsk = page.getByRole('button', { name: 'Ask AI about this', exact: true });
  await expect(cardAsk).toHaveCount(1);
  await cardAsk.click();
  await expect(page.getByText('scope-on', { exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => JSON.stringify((window as any).__lastAgentArgs.userArtifacts))).toContain('fixture-index-revision');
  await page.screenshot({ path: '.artifacts/kb-ui-audit-2026-10-03-card-scope.png', fullPage: true });
});

test('failed searches show a persistent retry state without stale evidence', async ({ page }) => {
  await page.goto('/');
  await installAuditSearch(page);
  const input = page.getByPlaceholder('Search by keyword...');
  await input.fill('previous success');
  await expect(page.getByRole('button', { name: 'previous success evidence random', exact: true })).toBeVisible();
  await input.fill('forced failure');
  await expect.poll(() => page.evaluate(() => (window as any).__kbAuditCalls.some((call: any) => call.query === 'forced failure'))).toBe(true);
  await expect(input).toHaveValue('forced failure');
  await expect(page.getByTestId('search-error')).toContainText('Synthetic search failure');
  await expect(page.getByRole('button', { name: 'previous success evidence random', exact: true })).toHaveCount(0);
  await page.getByTestId('search-error').getByRole('button', { name: 'Retry' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__kbAuditCalls.filter((call: any) => call.query === 'forced failure').length)).toBe(2);
  await page.screenshot({ path: '.artifacts/kb-ui-audit-2026-10-03-failure.png', fullPage: true });
});

async function installPagedRanking(page: import('@playwright/test').Page) {
  await installAuditSearch(page);
  await page.evaluate(() => {
    const host = window as any;
    const invoke = host.__TAURI_INTERNALS__.invoke;
    host.__rankingMethod = 'semantic_cross_encoder';
    host.__pagingOffsets = [];
    host.__TAURI_INTERNALS__.invoke = async (command: string, args: any = {}) => {
      const result = await invoke(command, args);
      if (command !== 'search' && command !== 'hybrid_search') return result;
      const offset = Number(args.offset ?? 0);
      host.__pagingOffsets.push(offset);
      return {
        ...result,
        totalMatches: 45,
        candidateLimitReached: true,
        ranking: { method: host.__rankingMethod, candidates: 64, elapsedMs: 5, fallbackReason: host.__rankingMethod === 'lexical_rules' ? 'Fixture reranker unavailable' : null },
        evidenceCards: result.evidenceCards.map((card: any) => ({ ...card, documentTitle: `${host.__rankingMethod} result ${offset}` })),
      };
    };
  });
}

test('pagination restarts from the first page when ranking switches to fallback', async ({ page }) => {
  await page.goto('/');
  await installPagedRanking(page);
  await page.getByPlaceholder('Search by keyword...').fill('pagination');
  await expect(page.getByRole('button', { name: 'semantic_cross_encoder result 0', exact: true })).toBeVisible();
  await page.evaluate(() => { (window as any).__rankingMethod = 'lexical_rules'; });
  await page.getByRole('button', { name: 'Next', exact: true }).click();
  await expect(page.getByRole('button', { name: 'lexical_rules result 0', exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__pagingOffsets)).toEqual([0, 20, 0]);
  await expect(page.getByTestId('search-ranking-reset')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Previous', exact: true })).toBeDisabled();
});

test('bounded result sets tell the user to narrow the search', async ({ page }) => {
  await page.goto('/');
  await installPagedRanking(page);
  await page.getByPlaceholder('Search by keyword...').fill('pagination');
  await expect(page.getByTestId('search-candidate-limit')).toContainText('Narrow');
});

test('a late pagination restart cannot replace a newer query', async ({ page }) => {
  await page.goto('/');
  await installPagedRanking(page);
  await page.evaluate(() => {
    const host = window as any;
    const invoke = host.__TAURI_INTERNALS__.invoke;
    host.__TAURI_INTERNALS__.invoke = async (command: string, args: any = {}) => {
      const result = await invoke(command, args);
      if (command !== 'search' && command !== 'hybrid_search') return result;
      const named = { ...result, evidenceCards: result.evidenceCards.map((card: any) => ({ ...card, documentTitle: `${args.queryText} ${card.documentTitle}` })) };
      if (host.__deferPageReset && args.queryText === 'older pages' && args.offset === 0) {
        return new Promise(resolve => { host.__resolvePageReset = () => resolve(named); });
      }
      return named;
    };
  });
  const input = page.getByPlaceholder('Search by keyword...');
  await input.fill('older pages');
  await expect(page.getByRole('button', { name: 'older pages semantic_cross_encoder result 0', exact: true })).toBeVisible();
  await page.evaluate(() => { const host = window as any; host.__deferPageReset = true; host.__rankingMethod = 'lexical_rules'; });
  await page.getByRole('button', { name: 'Next', exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof (window as any).__resolvePageReset)).toBe('function');
  await input.fill('newer pages');
  await expect(page.getByRole('button', { name: 'newer pages lexical_rules result 0', exact: true })).toBeVisible();
  await page.evaluate(() => (window as any).__resolvePageReset());
  await expect(page.getByRole('button', { name: 'newer pages lexical_rules result 0', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'older pages lexical_rules result 0', exact: true })).toHaveCount(0);
  await expect(page.getByTestId('search-ranking-reset')).toHaveCount(0);
});

test('clearing input invalidates an in-flight request and its loading state', async ({ page }) => {
  await page.goto('/');
  await installAuditSearch(page, true);
  const input = page.getByPlaceholder('Search by keyword...');
  await input.fill('pending request');
  await expect.poll(() => page.evaluate(() => (window as any).__kbAuditCalls.length)).toBe(1);
  await input.fill('');
  await page.evaluate(() => (window as any).__kbAuditPending['pending request']());
  await expect(page.getByRole('button', { name: 'pending request evidence random', exact: true })).toHaveCount(0);
  await expect(page.getByTestId('search-error')).toHaveCount(0);
  await expect(page.getByRole('main').getByRole('button', { name: 'Search', exact: true })).toBeEnabled();
});

test('evidence actions expose labels and keyboard descriptions', async ({ page }) => {
  await page.goto('/');
  await installAuditSearch(page);
  await page.getByPlaceholder('Search by keyword...').fill('keyboard evidence');
  const ask = page.getByRole('button', { name: 'Ask AI about this', exact: true });
  await expect(ask).toBeVisible();
  await ask.focus();
  await expect(page.getByRole('tooltip', { name: 'Ask AI about this' })).toBeVisible();
  await expect(ask).toHaveAttribute('aria-describedby', /.+/);
  await ask.press('Escape');
  await expect(page.getByRole('tooltip')).toHaveCount(0);
});

test('runtime indexing snapshots survive route remount and reject stale revisions', async ({ page }) => {
  await page.addInitScript(() => {
    const host = window as any;
    const invoke = host.__TAURI_INTERNALS__.invoke;
    const job = {
      id: 'scan-fixture', kind: 'scan-all', sourceId: null, status: 'running', error: null,
      progress: { operation: 'scan-all', sourceIndex: 1, sourceCount: 2, sourceId: 'source-retries', phase: 'parsing', current: 3, total: 10, currentFile: 'bilingual-report.pdf' },
      revision: 2, startedAt: '2026-10-03T12:00:00Z', finishedAt: null,
    };
    host.__fixtureKnowledgeJob = job;
    host.__TAURI_INTERNALS__.invoke = (command: string, args: unknown) => {
      if (command === 'list_knowledge_jobs') return Promise.resolve([job]);
      if (command === 'get_scan_errors_cmd') return Promise.resolve([]);
      return invoke(command, args);
    };
  });
  await page.goto('/sources');
  await expect(page.getByText('Scanning all sources...', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Scan All', exact: true })).toBeDisabled();
  await page.getByTestId('app-navigation-rail').getByRole('button', { name: 'Search', exact: true }).click();
  await expect(page.getByPlaceholder('Search by keyword...')).toBeVisible();
  await page.getByTestId('app-navigation-rail').getByRole('button', { name: 'Sources', exact: true }).click();
  await expect(page.getByText('Scanning all sources...', { exact: true })).toBeVisible();
  await page.evaluate(() => {
    const host = window as any;
    host.__emitKnowledgeFixture('knowledge:job', { ...host.__fixtureKnowledgeJob, status: 'completed', revision: 3 });
    host.__emitKnowledgeFixture('knowledge:job', host.__fixtureKnowledgeJob);
  });
  await expect(page.getByText('Scanning all sources...', { exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Scan All', exact: true })).toBeEnabled();
});

test('returning from a follow-up restores and refreshes the search workspace', async ({ page }) => {
  await page.goto('/');
  await installAuditSearch(page);
  await page.locator('button').filter({ hasText: 'Filters' }).click();
  await page.getByRole('button', { name: 'retries', exact: true }).click();
  const query = page.getByPlaceholder('Search by keyword...');
  await query.fill('retained workspace');
  await expect(page.getByRole('button', { name: 'retained workspace evidence retries', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Ask AI about this', exact: true }).click();
  await expect(page.getByText('scope-on', { exact: true })).toBeVisible();
  await page.getByTestId('app-navigation-rail').getByRole('button', { name: 'Search', exact: true }).click();
  await expect(query).toHaveValue('retained workspace');
  await expect(page.getByRole('button', { name: 'retained workspace evidence retries', exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__kbAuditCalls.length)).toBe(2);
});

test('research failures cannot become source indexing retries', async ({page}) => {
  await page.addInitScript(() => {
    const host=window as any; const invoke=host.__TAURI_INTERNALS__.invoke;
    host.__TAURI_INTERNALS__.invoke=(command:string,args:any)=> {
      if(command==='list_knowledge_jobs')return Promise.resolve([{id:'research-failure',kind:'research',sourceId:null,status:'failed',error:'research-failure-marker',progress:{setId:'saved-research'},revision:2,startedAt:new Date().toISOString(),finishedAt:new Date().toISOString()}]);
      if(command==='get_source_index_health')return Promise.resolve([]);
      return invoke(command,args);
    };
  });
  await page.goto('/sources');
  await expect(page.getByRole('button',{name:'Scan All',exact:true})).toBeEnabled();
  await expect(page.getByText('research-failure-marker',{exact:false})).toHaveCount(0);
  await page.evaluate(() => (window as any).__emitKnowledgeFixture('knowledge:job',{id:'research-active',kind:'research',sourceId:null,status:'running',error:null,progress:{setId:'saved-research',current:1,total:6},revision:1,startedAt:new Date().toISOString(),finishedAt:null}));
  await expect(page.getByRole('button',{name:'Scan All',exact:true})).toBeEnabled();
});
