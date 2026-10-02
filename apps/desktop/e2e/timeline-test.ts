import { test as base } from '@playwright/test';
export { expect } from '@playwright/test';
export type { Locator } from '@playwright/test';

/** Compatibility for pre-pagination presentation fixtures. These small mocks
 * intentionally supply their complete old transcript; pagination contracts live
 * in conversation-timeline.spec.ts and implement page/detail IPC directly. */
export const test = base.extend<{ timelineFixture: void }>({
  timelineFixture: [async ({ page }, use) => {
    await page.addInitScript(() => {
      type Internal = { invoke: (command: string, args?: Record<string,unknown>) => Promise<any> };
      const attach = (internal: Internal | undefined) => {
        if (!internal || (internal as any).__timelineFixture) return internal;
        (internal as any).__timelineFixture = true;
        const invoke = internal.invoke;
        internal.invoke = async (command,args = {}) => {
          if (command !== 'get_conversation_timeline_page_cmd') return invoke(command,args);
          // Use the current delegate so a test's later targeted mock overrides
          // remain visible. Old commands bypass this adapter and cannot recurse.
          const [conversation,messages] = await internal.invoke('get_conversation_cmd',{ id: args.conversationId });
          const turns = await internal.invoke('get_conversation_turns_cmd',{ conversationId: args.conversationId }) ?? [];
          const taskRuns = await internal.invoke('get_agent_task_runs_cmd',{ conversationId: args.conversationId }) ?? [];
          const roots = messages.filter((message: any) => message.role === 'user' && !['steering','questionResponse','checkpointContinuation'].includes(message.artifacts?.kind));
          const cursor = (message: any) => message ? { sortOrder: message.sortOrder, messageId: message.id } : null;
          return { conversation, messages, turns, taskRuns, entries: [], range: messages.length ? { from: cursor(messages[0]), before: null } : null, oldestCursor: cursor(roots[0]), newestCursor: cursor(roots[roots.length-1]), hasMoreBefore: false, hasMoreAfter: false };
        };
        return internal;
      };
      let current = attach((window as any).__TAURI_INTERNALS__);
      Object.defineProperty(window,'__TAURI_INTERNALS__',{ configurable: true, get: () => current, set: value => { current = attach(value); } });
    });
    await use();
  }, { auto: true }],
});
