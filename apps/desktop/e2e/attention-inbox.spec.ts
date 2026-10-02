import { expect, test } from './timeline-test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale', 'en');
    const now = new Date().toISOString();
    const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value));
    const conversations = ['alpha', 'beta'].map(id => ({
      id, title: `${id} chat`, provider: 'open_ai', model: 'gpt-4.1', systemPrompt: '',
      projectId: null, personaId: null, initialAutoTitlePending: false, archivedAt: null, createdAt: now, updatedAt: now,
    }));
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, { event: string; handler: number }>();
    let sequence = 0;
    let eventSequence = 0;
    let failDecision = true;
    const approval = {
      id: 'approval-alpha', toolName: 'run_shell', permissionKey: 'test-key', targetKind: 'shell', targetValue: 'workspace',
      argumentsPreview: '{}', riskLevel: 'high', reason: 'Run the requested verification command.',
      createdAt: now, expiresAt: new Date(Date.now() + 60_000).toISOString(),
    };
    let pending: unknown[] = new URLSearchParams(location.search).has('restore')
      ? [{ conversationId: 'alpha', runId: 'run-alpha', request: approval }] : [];
    const decisions: string[] = [];
    const emit = (kind: string, payload: unknown) => {
      const envelope = { conversationId: 'alpha', runEvent: {
        version: 2, runId: 'run-alpha', turnId: 'turn-alpha', eventSeq: ++eventSequence,
        kind, phase: 'approval', visibility: 'user', persistence: 'durable', displayKind: 'approval', importance: 'high',
        label: 'Approval', payload, createdAt: now,
      } };
      for (const [id, listener] of listeners) {
        if (listener.event === 'agent://run-event') callbacks.get(listener.handler)?.({ event: listener.event, id, payload: envelope });
      }
    };
    const config = { id: 'config', name: 'Test model', provider: 'open_ai', apiKey: '', baseUrl: null, model: 'gpt-4.1', temperature: 0.3, maxTokens: 4096, contextWindow: 100000, isDefault: true, createdAt: now, updatedAt: now };
    const invoke = async (cmd: string, args: Record<string, unknown> = {}) => {
      switch (cmd) {
        case 'plugin:event|listen': { const id = ++sequence; listeners.set(id, { event: String(args.event), handler: Number(args.handler) }); return id; }
        case 'plugin:event|unlisten': listeners.delete(Number(args.eventId)); return null;
        case 'get_wizard_state_cmd': return { completed: true, language: 'en' };
        case 'list_agent_configs_cmd': return [config];
        case 'get_model_context_window': return 100000;
        case 'list_conversations_cmd': return clone(conversations);
        case 'get_conversation_cmd': return [clone(conversations.find(c => c.id === args.id)), []];
        case 'list_pending_tool_approvals_cmd': return clone(pending);
        case 'approve_tool_call_cmd':
          decisions.push(String(args.requestId));
          if (failDecision) { failDecision = false; throw new Error('Host temporarily unavailable'); }
          pending = [];
          emit('approvalResolved', { requestId: approval.id });
          return null;
        case 'get_conversation_turns_cmd': case 'get_agent_task_runs_cmd': case 'list_interaction_requests_cmd':
        case 'list_sources': case 'list_projects_cmd': case 'list_personas_cmd': case 'list_checkpoints_cmd':
        case 'list_skills_cmd': case 'list_mcp_servers_cmd': case 'list_user_memories_cmd': case 'get_conversation_sources_cmd': return [];
        default: return null;
      }
    };
    Object.assign(window, {
      __ATTENTION_TEST__: {
        decisions,
        request: () => { pending = [{ conversationId: 'alpha', runId: 'run-alpha', request: approval }]; emit('approvalRequested', { request: approval }); },
        expire: () => { approval.expiresAt = new Date(Date.now() - 1000).toISOString(); document.dispatchEvent(new Event('visibilitychange')); },
        clear: () => { pending = []; document.dispatchEvent(new Event('visibilitychange')); },
      },
      __TAURI_INTERNALS__: {
        invoke, metadata: { currentWindow: { label: 'main' } }, convertFileSrc: (path: string) => path,
        transformCallback: (callback: (event: unknown) => void) => { const id = ++sequence; callbacks.set(id, callback); return id; },
        unregisterCallback: (id: number) => callbacks.delete(id),
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: (_event: string, id: number) => listeners.delete(id) },
    });
  });
});

test('background approval remains discoverable across routes, and failed submission can be retried', async ({ page }) => {
  await page.goto('/chat/beta');
  await expect(page.getByTestId('conversation-item-alpha')).toBeVisible();
  await page.evaluate(() => (window as any).__ATTENTION_TEST__.request());
  await expect(page.getByTestId('conversation-approval-alpha')).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByTestId('attention-inbox-count')).toHaveText('1');
  await page.getByRole('link', { name: 'Settings', exact: true }).click();
  await page.getByTestId('attention-inbox-toggle').click();
  await page.getByTestId('attention-item-approval:approval-alpha').click();
  await expect(page).toHaveURL(/\/chat\/alpha$/);
  const dialog = page.getByRole('dialog', { name: 'run_shell', exact: true });
  await expect(dialog).toContainText('Run the requested verification command.');
  await expect(page.getByTestId('approval-deadline')).toContainText('Respond within');
  await dialog.getByRole('button', { name: 'Allow Once', exact: true }).click();
  await expect(page.getByTestId('approval-submit-error')).toContainText('Host temporarily unavailable');
  await dialog.getByRole('button', { name: 'Allow Once', exact: true }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByTestId('attention-inbox-count')).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__ATTENTION_TEST__.decisions)).toEqual(['approval-alpha', 'approval-alpha']);
});

test('pending host snapshot restores outside chat, expiry disables decisions and host resolution clears the count', async ({ page }) => {
  await page.goto('/settings?restore=1');
  await expect(page.getByTestId('attention-inbox-count')).toHaveText('1');
  await page.evaluate(() => (window as any).__ATTENTION_TEST__.expire());
  await page.getByTestId('attention-inbox-toggle').click();
  await page.getByTestId('attention-item-approval:approval-alpha').click();
  const dialog = page.getByRole('dialog', { name: 'run_shell', exact: true });
  await expect(page.getByTestId('approval-deadline')).toContainText('expired');
  await expect(dialog.getByRole('button', { name: 'Allow Once', exact: true })).toBeDisabled();
  await page.evaluate(() => (window as any).__ATTENTION_TEST__.clear());
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByTestId('attention-inbox-count')).toHaveCount(0);
});
