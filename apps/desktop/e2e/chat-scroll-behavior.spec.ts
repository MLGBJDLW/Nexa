import { expect, test } from '@playwright/test';
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
    let seq = 0;
    let streamedReplyCount = 0;
    const sharedScreen = { frames: 0, stopped: 0, nativeCaptures: [] as string[], streams: [] as MediaStream[] };
    Object.assign(window, { __sharedScreen: sharedScreen });
    Object.defineProperty(navigator.mediaDevices, 'getDisplayMedia', { configurable: true, value: async () => {
      const canvas = document.createElement('canvas');
      canvas.width = 640; canvas.height = 360;
      const context = canvas.getContext('2d')!;
      context.fillStyle = '#157d8c'; context.fillRect(0, 0, 640, 360);
      const stream = canvas.captureStream(2);
      sharedScreen.streams.push(stream);
      return stream;
    } });
    const nextId = (prefix: string) => `${prefix}-${Date.now()}-${seq++}`;
    const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

    const longLine = 'This is a long paragraph to force the chat timeline to overflow and require scrolling inside the log container.';
    const footnoteMessage = [
      'The opening claim cites a source right away.[^1]',
      '',
      ...Array.from({ length: 18 }, (_, idx) => `Paragraph ${idx + 1}: ${longLine} ${longLine}`),
      '',
      '[^1]: Evidence section rendered near the end of the message so the jump target sits much lower than the reference.',
    ].join('\n\n');

    const buildScrollableHistory = (conversationId: string): Message[] => {
      const items: Message[] = [];
      for (let i = 0; i < 16; i += 1) {
        items.push({
          id: nextId('m-user'),
          conversationId,
          role: 'user',
          content: `Question ${i + 1}: ${longLine}`,
          toolCallId: null,
          toolCalls: [],
          artifacts: null,
          tokenCount: 0,
          createdAt: new Date().toISOString(),
          sortOrder: items.length,
          thinking: null,
          imageAttachments: null,
        });
        items.push({
          id: nextId('m-assistant'),
          conversationId,
          role: 'assistant',
          content: `Answer ${i + 1}: ${longLine} ${longLine}`,
          toolCallId: null,
          toolCalls: [],
          artifacts: null,
          tokenCount: 0,
          createdAt: new Date().toISOString(),
          sortOrder: items.length,
          thinking: null,
          imageAttachments: null,
        });
      }
      return items;
    };

    const conversations: Record<string, Conversation> = {
      'conv-footnotes': {
        id: 'conv-footnotes',
        title: 'Footnote Scroll',
        provider: 'open_ai',
        model: 'gpt-4.1',
        systemPrompt: '',
        createdAt: nowIso,
        updatedAt: nowIso,
      },
      'conv-auto-follow': {
        id: 'conv-auto-follow',
        title: 'Auto Follow',
        provider: 'open_ai',
        model: 'gpt-4.1',
        systemPrompt: '',
        createdAt: nowIso,
        updatedAt: nowIso,
      },
    };

    const messagesByConversation: Record<string, Message[]> = {
      'conv-footnotes': [
        {
          id: nextId('m-footnote-assistant'),
          conversationId: 'conv-footnotes',
          role: 'assistant',
          content: footnoteMessage,
          toolCallId: null,
          toolCalls: [],
          artifacts: null,
          tokenCount: 0,
          createdAt: nowIso,
          sortOrder: 0,
          thinking: null,
          imageAttachments: null,
        },
      ],
      'conv-auto-follow': buildScrollableHistory('conv-auto-follow'),
    };

    const callbackMap = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, { event: string; handlerId: number }>();
    let callbackSeq = 1;
    let listenerSeq = 1;

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
          callback({
            event: eventName,
            id: listenerId,
            payload,
          });
        }
      }
    };

    const defaultAgentConfig = {
      id: 'cfg-scroll-behavior',
      name: 'Scroll Behavior Config',
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
        case 'list_desktop_monitors_cmd': return [{ id: 'monitor-left', width: 1920, height: 1080, primary: true }, { id: 'monitor-right', width: 2560, height: 1440, primary: false }];
        case 'capture_desktop_monitor_cmd': {
          sharedScreen.nativeCaptures.push(String(args.monitorId));
          const canvas = document.createElement('canvas'); canvas.width = 640; canvas.height = 360;
          const context = canvas.getContext('2d')!; context.fillStyle = '#123456'; context.fillRect(0, 0, 640, 360);
          return canvas.toDataURL('image/jpeg').split(',')[1];
        }
        case 'begin_desktop_share_cmd': return 'fixture-share';
        case 'update_desktop_share_cmd': sharedScreen.frames++; return null;
        case 'end_desktop_share_cmd': sharedScreen.stopped++; return null;
        case 'plugin:event|listen': {
          const listenerId = listenerSeq++;
          listeners.set(listenerId, {
            event: String(args.event ?? ''),
            handlerId: Number(args.handler ?? 0),
          });
          return listenerId;
        }
        case 'plugin:event|unlisten': {
          listeners.delete(Number(args.eventId ?? 0));
          return null;
        }
        case 'list_agent_configs_cmd':
          return [clone(defaultAgentConfig)];
        case 'get_model_context_window':
          return 1047576;
        case 'list_conversations_cmd':
          return Object.values(conversations).map(clone);
        case 'get_conversation_cmd': {
          const id = String(args.id ?? '');
          return [clone(conversations[id]), clone(messagesByConversation[id] ?? [])];
        }
        case 'list_sources':
          return [];
        case 'get_conversation_sources_cmd':
          return [];
        case 'set_conversation_sources_cmd':
          return null;
        case 'update_conversation_system_prompt_cmd':
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
        case 'list_user_memories_cmd':
          return [];
        case 'list_skills_cmd':
          return [];
        case 'list_mcp_servers_cmd':
          return [];
          return 0;
        case 'agent_chat_cmd': {
          const conversationId = String(args.conversationId ?? '');
          if (conversationId !== 'conv-auto-follow') {
            return null;
          }

          streamedReplyCount += 1;
          const currentMessages = messagesByConversation[conversationId] ?? [];
          const userText = String(args.message ?? '');
          const assistantText = `Streamed answer #${streamedReplyCount}`;

          const userMessage: Message = {
            id: nextId('m-user-live'),
            conversationId,
            role: 'user',
            content: userText,
            toolCallId: null,
            toolCalls: [],
            artifacts: null,
            tokenCount: 0,
            createdAt: new Date().toISOString(),
            sortOrder: currentMessages.length,
            thinking: null,
            imageAttachments: null,
          };

          const assistantMessage: Message = {
            id: nextId('m-assistant-live'),
            conversationId,
            role: 'assistant',
            content: assistantText,
            toolCallId: null,
            toolCalls: [],
            artifacts: null,
            tokenCount: 0,
            createdAt: new Date().toISOString(),
            sortOrder: currentMessages.length + 1,
            thinking: null,
            imageAttachments: null,
          };

          messagesByConversation[conversationId] = [
            ...currentMessages,
            userMessage,
            assistantMessage,
          ];
          conversations[conversationId].updatedAt = new Date().toISOString();

          setTimeout(() => {
            emitEvent('agent://run-event', {
              conversationId,
              type: 'textDelta',
              delta: assistantText,
            });
          }, 40);

          setTimeout(() => {
            emitEvent('agent://run-event', {
              conversationId,
              type: 'done',
              message: assistantMessage,
              usageTotal: {
                promptTokens: 600,
                completionTokens: 120,
                totalTokens: 720,
                thinkingTokens: 0,
              },
              lastPromptTokens: 600,
              finishReason: 'stop',
              cached: false,
            });
          }, 90);

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

    (window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__: unknown }).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: (_event: string, eventId: number) => {
        listeners.delete(eventId);
      },
    };
  });
});

test('keeps footnote jumps inside the chat scroller', async ({ page }) => {
  await page.goto('/chat/conv-footnotes');

  const scrollRoot = page.locator('[data-chat-scroll-root="true"]');
  await scrollRoot.evaluate((el) => {
    el.scrollTop = 0;
    el.dispatchEvent(new Event('scroll'));
  });

  const footnoteRef = page.locator('a[href="#user-content-fn-1"], a[href="#fn-1"]').first();
  await expect(footnoteRef).toBeVisible();

  const initialHash = await page.evaluate(() => window.location.hash);
  await footnoteRef.click();

  await expect.poll(async () => scrollRoot.evaluate((el) => el.scrollTop)).toBeGreaterThan(80);
  await expect.poll(async () => page.locator('#user-content-fn-1, #fn-1').evaluate((target) => {
    const root = target.closest('[data-chat-scroll-root="true"]') as HTMLElement | null;
    if (!root) return false;
    const rootRect = root.getBoundingClientRect();
    const targetRect = target.getBoundingClientRect();
    const top = targetRect.top - rootRect.top;
    const bottom = targetRect.bottom - rootRect.top;
    return top < root.clientHeight && bottom > 0;
  })).toBe(true);

  expect(await page.evaluate(() => window.location.hash)).toBe(initialHash);
});

test('auto-follows only while the user stays near the bottom', async ({ page }) => {
  await page.goto('/chat/conv-auto-follow');

  const scrollRoot = page.locator('[data-chat-scroll-root="true"]');
  await expect(scrollRoot).toBeVisible();

  await expect.poll(async () => scrollRoot.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(32);

  await page.getByTestId('chat-input-textarea').fill('Send the next update.');
  await page.getByTestId('chat-send').click();

  await expect(page.getByText('Streamed answer #1')).toBeVisible();
  await expect.poll(async () => scrollRoot.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(32);

  await scrollRoot.evaluate((el) => {
    el.scrollTop = Math.max(0, (el.scrollHeight - el.clientHeight) / 2);
    el.dispatchEvent(new Event('scroll'));
  });

  await expect.poll(async () => scrollRoot.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeGreaterThan(80);

  await page.getByTestId('chat-input-textarea').fill('Send one more update.');
  await page.getByTestId('chat-send').click();

  // The completed answer may be virtualized while the user reads older turns.
  await expect(page.getByTestId('chat-send')).toBeVisible();
  await expect.poll(async () => scrollRoot.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeGreaterThan(80);

  const scrollToBottom = page.getByTitle('Scroll to bottom');
  await expect(scrollToBottom).toBeVisible();
  await scrollToBottom.click();

  await expect.poll(async () => scrollRoot.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(32);
  await expect(page.getByText('Streamed answer #2')).toBeVisible();
});

test('follows delayed layout growth without mistaking it for user scrolling', async ({ page }) => {
  await page.goto('/chat/conv-auto-follow');
  const root = page.locator('[data-chat-scroll-root="true"]');
  await expect.poll(() => root.evaluate(el => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(3);
  await root.evaluate(el => {
    const content = el.querySelector<HTMLElement>('[data-chat-follow-content="true"]')!;
    content.style.paddingBottom = '600px';
    el.dispatchEvent(new Event('scroll'));
  });
  await expect.poll(() => root.evaluate(el => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(3);
  await root.hover();
  await page.mouse.wheel(0, -400);
  await expect.poll(() => root.evaluate(el => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeGreaterThan(200);
  const readingTop = await root.evaluate(el => el.scrollTop);
  await root.locator('[data-chat-follow-content="true"]').evaluate(el => { (el as HTMLElement).style.paddingBottom = '900px'; });
  await expect.poll(() => root.evaluate(el => el.scrollTop)).toBe(readingTop);
});

test('explicit monitor selection shares the whole selected display and stops native capture', async ({ page }) => {
  await page.addInitScript(() => { Object.assign(window, { isTauri: true }); });
  await page.goto('/chat/conv-auto-follow');
  const captures = () => page.evaluate(() => (window as unknown as { __sharedScreen: { nativeCaptures: string[] } }).__sharedScreen.nativeCaptures);
  await page.getByTestId('desktop-share-toggle').click();
  await expect(page.getByRole('button', { name: /Screen 2.*2560/ })).toBeVisible();
  expect(await captures()).toEqual([]);
  await page.getByRole('button', { name: /Screen 2.*2560/ }).click();
  await expect(page.getByTestId('desktop-share-toggle')).toHaveAttribute('aria-pressed', 'true');
  await expect.poll(async () => (await captures()).length).toBeGreaterThan(1);
  expect((await captures()).every(id => id === 'monitor-right')).toBe(true);
  await page.getByTestId('desktop-share-toggle').click();
  const count = (await captures()).length;
  await page.waitForTimeout(1200);
  expect(await captures()).toHaveLength(count);
});

test('disables screen sharing when neither browser nor native display capture is available', async ({ page }) => {
  await page.goto('/chat/conv-auto-follow');
  await page.evaluate(() => {
    Object.defineProperty(navigator.mediaDevices, 'getDisplayMedia', { value: undefined });
    const runtime = (window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args?: unknown) => Promise<unknown> } }).__TAURI_INTERNALS__;
    const invoke = runtime.invoke;
    runtime.invoke = (command, args) => command === 'list_desktop_monitors_cmd' ? Promise.resolve([]) : invoke(command, args);
    Object.assign(window, { isTauri: true });
  });
  await page.getByText('Footnote Scroll', { exact: true }).click();
  await expect(page.getByTestId('desktop-share-toggle')).toBeDisabled();
});

test('screen sharing sends fresh frames and stops on revocation or conversation change', async ({ page }) => {
  await page.goto('/chat/conv-auto-follow');
  const share = page.getByTestId('desktop-share-toggle');
  const state = () => page.evaluate(() => {
    const shared = (window as unknown as { __sharedScreen: { frames: number; stopped: number; streams: MediaStream[] } }).__sharedScreen;
    return { frames: shared.frames, stopped: shared.stopped, tracks: shared.streams.flatMap(stream => stream.getTracks().map(track => track.readyState)) };
  });
  await share.click();
  await page.getByRole('button', { name: 'Window / system picker…' }).click();
  await expect(share).toHaveAttribute('aria-pressed', 'true');
  await expect.poll(async () => (await state()).frames).toBeGreaterThan(1);
  await share.click();
  await expect.poll(async () => (await state()).tracks.every(status => status === 'ended')).toBe(true);
  await expect.poll(async () => (await state()).stopped).toBe(1);
  const stoppedFrames = (await state()).frames;
  await page.waitForTimeout(1100);
  expect((await state()).frames).toBe(stoppedFrames);
  await share.click();
  await page.getByRole('button', { name: 'Window / system picker…' }).click();
  await expect(share).toHaveAttribute('aria-pressed', 'true');
  // A client-side navigation must release capture without relying on a page unload.
  await page.getByText('Footnote Scroll', { exact: true }).click();
  await expect.poll(async () => (await state()).tracks.every(status => status === 'ended')).toBe(true);
  await expect(share).toHaveAttribute('aria-pressed', 'false');
});

test('stopping a pending screen share releases media before its lease arrives', async ({ page }) => {
  await page.goto('/chat/conv-auto-follow');
  await page.evaluate(() => {
    const runtime = (window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args?: unknown) => Promise<unknown> } }).__TAURI_INTERNALS__;
    const invoke = runtime.invoke;
    runtime.invoke = (command, args) => command === 'begin_desktop_share_cmd'
      ? new Promise(resolve => { Object.assign(window, { __finishScreenLease: () => resolve('late-lease') }); })
      : invoke(command, args);
  });
  const share = page.getByTestId('desktop-share-toggle');
  await share.click();
  await page.getByRole('button', { name: 'Window / system picker…' }).click();
  await expect.poll(() => page.evaluate(() => '__finishScreenLease' in window)).toBe(true);
  await share.click();
  await expect.poll(() => page.evaluate(() => {
    const shared = (window as unknown as { __sharedScreen: { streams: MediaStream[] } }).__sharedScreen;
    return shared.streams.length > 0 && shared.streams.every(stream => stream.getTracks().every(track => track.readyState === 'ended'));
  })).toBe(true);
  await page.evaluate(() => (window as unknown as { __finishScreenLease: () => void }).__finishScreenLease());
  await expect.poll(() => page.evaluate(() => (window as unknown as { __sharedScreen: { stopped: number } }).__sharedScreen.stopped)).toBe(1);
  expect(await page.evaluate(() => (window as unknown as { __sharedScreen: { frames: number } }).__sharedScreen.frames)).toBe(0);
  await expect(share).toHaveAttribute('aria-pressed', 'false');
});

test('bounds a high-entropy screen frame and keeps its JPEG decodable', async ({ page }) => {
  await page.goto('/chat/conv-auto-follow');
  const result = await page.evaluate(async () => {
    const modulePath = '/src/lib/sharedScreenFrame.ts';
    const { encodeSharedScreenFrame, MAX_SHARED_SCREEN_BASE64 } = await import(/* @vite-ignore */ modulePath);
    const source = document.createElement('canvas');
    source.width = source.height = 1568;
    const context = source.getContext('2d')!;
    const pixels = context.createImageData(source.width, source.height);
    let seed = 13579;
    for (let i = 0; i < pixels.data.length; i += 4) {
      seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5;
      pixels.data[i] = seed & 255;
      pixels.data[i + 1] = (seed >>> 8) & 255;
      pixels.data[i + 2] = (seed >>> 16) & 255;
      pixels.data[i + 3] = 255;
    }
    context.putImageData(pixels, 0, 0);
    const originalLength = source.toDataURL('image/jpeg', 0.65).split(',')[1].length;
    const frame = encodeSharedScreenFrame(source, source.width, source.height, document.createElement('canvas'))!;
    const image = new Image(); image.src = frame.url; await image.decode();
    return { originalLength, length: frame.base64.length, limit: MAX_SHARED_SCREEN_BASE64, width: image.naturalWidth, height: image.naturalHeight };
  });
  expect(result.originalLength).toBeGreaterThan(result.limit);
  expect(result.length).toBeLessThanOrEqual(result.limit);
  expect(result.length).toBeGreaterThan(0);
  expect(Math.max(result.width, result.height)).toBeLessThanOrEqual(1568);
});

test('a frame that cannot fit does not stop screen sharing', async ({ page }) => {
  await page.goto('/chat/conv-auto-follow');
  await page.evaluate(() => {
    const encode = HTMLCanvasElement.prototype.toDataURL;
    let attempts = 0;
    HTMLCanvasElement.prototype.toDataURL = function(type, quality) {
      return attempts++ < 9 ? `data:image/jpeg;base64,${'A'.repeat(1_400_004)}` : encode.call(this, type, quality);
    };
  });
  await page.getByTestId('desktop-share-toggle').click();
  await page.getByRole('button', { name: 'Window / system picker…' }).click();
  await expect(page.getByTestId('desktop-share-toggle')).toHaveAttribute('aria-pressed', 'true');
  await expect.poll(() => page.evaluate(() => (window as unknown as { __sharedScreen: { frames: number } }).__sharedScreen.frames)).toBeGreaterThan(1);
  expect(await page.evaluate(() => (window as unknown as { __sharedScreen: { stopped: number } }).__sharedScreen.stopped)).toBe(0);
});
