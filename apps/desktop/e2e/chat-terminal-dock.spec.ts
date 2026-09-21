import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale', 'en');

    type Conversation = {
      id: string;
      title: string;
      provider: string;
      model: string;
      systemPrompt: string;
      collectionContext: null;
      projectId: null;
      createdAt: string;
      updatedAt: string;
    };

    const nowIso = new Date().toISOString();
    const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

    const conversation: Conversation = {
      id: 'conv-terminal-dock',
      title: 'Terminal dock',
      provider: 'open_ai',
      model: 'gpt-4.1',
      systemPrompt: '',
      collectionContext: null,
      projectId: null,
      createdAt: nowIso,
      updatedAt: nowIso,
    };
    const race = new URL(location.href).searchParams.get('terminalRace');
    const otherConversation = { ...conversation, id: 'conv-terminal-other', title: 'Other terminal conversation' };
    const otherSession = {
      id: 'terminal-other', shell: 'PowerShell', cwd: 'D:\\project-B', processId: 4243,
      conversationId: otherConversation.id,
    };
    const initialSession = {
      id: 'terminal-session-1', shell: 'PowerShell', cwd: 'D:\\project-A', processId: 4242,
      conversationId: conversation.id,
    };
    const alternateSession = { ...initialSession, id: 'terminal-session-2', processId: 4244 };
    const pendingCommands = new Map<string, () => void>();
    const hold = (key: string) => new Promise<void>((resolve) => pendingCommands.set(key, resolve));

    const callbackMap = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, { event: string; handlerId: number }>();
    let callbackSeq = 1;
    let listenerSeq = 1;

    const terminalDiagnostics = {
      starts: [] as Array<Record<string, unknown>>,
      writes: [] as string[],
      resizes: [] as Array<Record<string, unknown>>,
      closes: [] as string[],
      bindings: [] as Array<Record<string, unknown>>,
      writeSessions: [] as string[],
      pending: [] as string[],
    };
    if (race === 'paste') Object.defineProperty(navigator, 'clipboard', { value: {
      readText: async () => {
        terminalDiagnostics.pending.push('clipboard');
        await hold('clipboard');
        return 'echo delayed paste';
      },
    } });

    const emitEvent = (eventName: string, payload: Record<string, unknown>) => {
      for (const [listenerId, listener] of listeners.entries()) {
        if (listener.event !== eventName) continue;
        const callback = callbackMap.get(listener.handlerId);
        if (callback) {
          callback({ event: eventName, id: listenerId, payload });
        }
      }
    };

    const defaultAgentConfig = {
      id: 'cfg-terminal-dock',
      name: 'Terminal Dock Config',
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
          return race ? [clone(conversation), clone(otherConversation)] : [clone(conversation)];
        case 'get_conversation_cmd':
          return [clone(args.id === otherConversation.id ? otherConversation : conversation), []];
        case 'get_agent_run_event_page_cmd':
          return {
            events: [],
            durableHighWater: Number(args.durableHighWater ?? args.afterEventSeq ?? 0),
            nextAfterEventSeq: null,
            hasMore: false,
          };
        case 'get_conversation_turns_cmd':
        case 'get_agent_task_runs_cmd':
        case 'get_agent_run_events_cmd':
        case 'get_agent_task_run_events_cmd':
        case 'list_sources':
        case 'get_conversation_sources_cmd':
        case 'list_checkpoints_cmd':
        case 'list_user_memories_cmd':
        case 'list_skills_cmd':
        case 'list_mcp_servers_cmd':
        case 'list_projects_cmd':
        case 'list_personas_cmd':
          return [];
        case 'terminal_start_session_cmd': {
          terminalDiagnostics.starts.push(clone(args));
          const session = {
            id: 'terminal-session-1',
            shell: 'PowerShell',
            cwd: 'D:\\Apps\\ask_myself',
            processId: 4242,
            conversationId: String((args.input as Record<string, unknown> | undefined)?.conversationId ?? ''),
          };
          if (race === 'start' && session.conversationId === conversation.id) {
            terminalDiagnostics.pending.push('start');
            await hold('start');
          }
          setTimeout(() => {
            emitEvent('terminal:event', {
              sessionId: session.id,
              kind: 'data',
              data: 'PS D:\\Apps\\ask_myself> ',
              exitCode: null,
              signal: null,
            });
          }, 50);
          return session;
        }
        case 'terminal_write_session_cmd':
          terminalDiagnostics.writes.push(String(args.data ?? ''));
          terminalDiagnostics.writeSessions.push(String(args.sessionId ?? ''));
          return null;
        case 'terminal_resize_session_cmd':
          terminalDiagnostics.resizes.push(clone(args));
          return null;
        case 'terminal_close_session_cmd':
          terminalDiagnostics.closes.push(String(args.sessionId ?? ''));
          if (race === 'restart') {
            terminalDiagnostics.pending.push('close');
            await hold('close');
          }
          return null;
        case 'terminal_bind_session_cmd':
          terminalDiagnostics.bindings.push(clone(args));
          if (race && args.sessionId === alternateSession.id) return clone(alternateSession);
          return {
            id: String(args.sessionId ?? 'terminal-session-1'),
            shell: 'PowerShell',
            cwd: 'D:\\Apps\\ask_myself',
            processId: 4242,
            conversationId: String(args.conversationId ?? ''),
          };
        case 'terminal_snapshot_session_cmd':
          if (args.sessionId === otherSession.id) return { session: clone(otherSession), output: 'B prompt> ' };
          if (race && args.sessionId === alternateSession.id) {
            terminalDiagnostics.pending.push('snapshot');
            await hold('snapshot');
            return { session: clone(alternateSession), output: 'A alternate prompt> ' };
          }
          return {
            session: {
              id: String(args.sessionId ?? 'terminal-session-1'),
              shell: 'PowerShell',
              cwd: 'D:\\Apps\\ask_myself',
              processId: 4242,
              conversationId: 'conv-terminal-dock',
            },
            output: 'PS D:\\Apps\\ask_myself> ',
          };
        case 'terminal_list_sessions_cmd':
          if (race === 'restore' && location.pathname.endsWith(otherConversation.id)) {
            terminalDiagnostics.pending.push('list');
            await hold('list');
          }
          return race === 'start' ? [clone(otherSession)]
            : race ? [clone(initialSession), clone(alternateSession), clone(otherSession)] : [];
        case 'terminal_active_session_cmd':
          return args.conversationId === otherConversation.id ? clone(otherSession)
            : race && race !== 'start' ? clone(initialSession) : null;
        case 'terminal_appearance_cmd':
          return {
            source: 'Windows Terminal', fontFamily: 'Consolas', fontSize: 18,
            theme: { background: '#fdf6e3', foreground: '#657b83', magenta: '#d33682', cyan: '#2aa198' },
            cursorStyle: 'bar', cursorBlink: false,
          };
        default:
          return null;
      }
    };

    (window as unknown as { __terminalDiagnostics__: unknown }).__terminalDiagnostics__ = terminalDiagnostics;
    (window as unknown as { __releaseTerminalCommand__: (key: string) => void }).__releaseTerminalCommand__ = (key) => {
      const resolve = pendingCommands.get(key);
      if (!resolve) throw new Error(`No pending terminal command: ${key}`);
      pendingCommands.delete(key);
      resolve();
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

test('terminal inherits the native profile font and palette', async ({ page }, testInfo) => {
  await page.goto('/chat/conv-terminal-dock');
  await page.getByRole('button', { name: 'Toggle terminal' }).click();
  const terminal = page.locator('.xterm');
  await expect(terminal).toBeVisible();
  await expect(page.getByTestId('terminal-screen')).toHaveCSS('font-family', /Consolas/);
  await expect(page.getByTestId('terminal-screen')).toHaveCSS('font-size', '18px');
  await expect(page.getByTestId('terminal-screen')).toHaveCSS('background-color', 'rgb(253, 246, 227)');
  await page.getByTestId('terminal-screen').screenshot({ path: testInfo.outputPath('terminal-native-theme.png') });
});

test('terminal remains interactive when WebGL is unavailable', async ({ page }) => {
  await page.addInitScript(() => {
    const original = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (kind: string, ...args: unknown[]) {
      if (kind === 'webgl' || kind === 'webgl2' || kind === 'experimental-webgl') return null;
      return Reflect.apply(original, this, [kind, ...args]);
    } as typeof original;
  });
  await page.goto('/chat/conv-terminal-dock');
  await page.getByRole('button', { name: 'Toggle terminal' }).click();
  await expect(page.locator('.xterm-rows')).toBeVisible();
  await expect(page.locator('.xterm-rows')).toHaveCSS('font-size', '18px');
  await expect(page.locator('.xterm-rows')).toContainText('PS D:');
  expect((await page.locator('.xterm-rows').innerText()).match(/PowerShell/g)).toHaveLength(1);
  await page.locator('.xterm-helper-textarea').focus();
  await page.keyboard.type('echo hello');
  await expect.poll(() => page.evaluate(() => (window as unknown as { __terminalDiagnostics__: { writes: string[] } }).__terminalDiagnostics__.writes.join(''))).toContain('echo hello');
});

for (const operation of ['start', 'switch', 'restart', 'paste'] as const) {
test(`interactive terminal dock: a delayed terminal ${operation} cannot replace another conversation terminal`, async ({ page }) => {
  await page.goto(`/chat/conv-terminal-dock?terminalRace=${operation}`);
  await page.getByRole('button', { name: 'Toggle terminal' }).click();
  const pendingCommand = operation === 'start' ? 'start' : operation === 'switch' ? 'snapshot'
    : operation === 'restart' ? 'close' : 'clipboard';
  if (operation !== 'start') {
    await expect(page.getByText(/^PowerShell #4242 ·/)).toBeVisible();
    if (operation === 'switch') {
      await page.getByRole('combobox', { name: 'Active terminal session' }).click();
      await page.getByRole('option', { name: '2: PowerShell #4244' }).click();
    } else if (operation === 'restart') {
      await page.getByRole('button', { name: 'Restart terminal' }).click();
    } else {
      await page.locator('.xterm-helper-textarea').focus();
      await page.keyboard.press('Control+Shift+V');
    }
  }
  await expect.poll(() => page.evaluate(() => (window as unknown as {
    __terminalDiagnostics__: { pending: string[] };
  }).__terminalDiagnostics__.pending)).toContain(pendingCommand);

  await page.getByText('Other terminal conversation', { exact: true }).click();
  await expect(page.getByText('PowerShell #4243')).toBeVisible();
  await page.evaluate((command) => (window as unknown as {
    __releaseTerminalCommand__: (key: string) => void;
  }).__releaseTerminalCommand__(command), pendingCommand);
  // Flush the released IPC promise and the React render it schedules.
  await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  await expect(page.getByText('PowerShell #4243')).toBeVisible();
  await expect(page.getByText('PowerShell #4242')).toHaveCount(0);
  expect(await page.evaluate(() => (window as unknown as {
    __terminalDiagnostics__: { writes: string[] };
  }).__terminalDiagnostics__.writes.filter(Boolean))).toEqual([]);
  await page.locator('.xterm-helper-textarea').focus();
  await page.keyboard.type('echo current');
  await expect.poll(() => page.evaluate(() => {
    const diagnostics = (window as unknown as {
      __terminalDiagnostics__: { writes: string[]; writeSessions: string[] };
    }).__terminalDiagnostics__;
    return diagnostics.writeSessions.filter((_, index) => Boolean(diagnostics.writes[index]));
  })).toEqual(Array('echo current'.length).fill('terminal-other'));
  expect(await page.evaluate(() => (window as unknown as {
    __terminalDiagnostics__: { closes: string[] };
  }).__terminalDiagnostics__.closes)).toEqual(operation === 'restart' ? ['terminal-session-1'] : []);
  expect(await page.evaluate(() => (window as unknown as {
    __terminalDiagnostics__: { starts: unknown[] };
  }).__terminalDiagnostics__.starts.length)).toBe(operation === 'start' ? 1 : 0);
});
}

test('interactive terminal dock: pending restoration does not offer another conversation terminal sessions', async ({ page }) => {
  await page.goto('/chat/conv-terminal-dock?terminalRace=restore');
  await page.getByRole('button', { name: 'Toggle terminal' }).click();
  await expect(page.getByRole('combobox', { name: 'Active terminal session' })).toBeVisible();
  await page.getByText('Other terminal conversation', { exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as {
    __terminalDiagnostics__: { pending: string[] };
  }).__terminalDiagnostics__.pending)).toContain('list');
  await expect(page.getByRole('combobox', { name: 'Active terminal session' })).toHaveCount(0);
  await page.evaluate(() => (window as unknown as {
    __releaseTerminalCommand__: (key: string) => void;
  }).__releaseTerminalCommand__('list'));
  await expect(page.getByText('PowerShell #4243')).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as {
    __terminalDiagnostics__: { bindings: unknown[] };
  }).__terminalDiagnostics__.bindings)).toEqual([]);
});

for (const initialWidth of [900, 1360]) {
  test(`interactive terminal dock fits without horizontal scrolling from ${initialWidth}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: initialWidth, height: 900 });
    await page.goto('/chat/conv-terminal-dock');
    await page.getByRole('button', { name: 'Toggle terminal' }).click();
    await expect(page.locator('.xterm')).toBeVisible();
    await expect(page.getByText('Running')).toBeVisible();
    await expect(page.getByTestId('terminal-screen')).toHaveCSS('font-size', '18px');

    for (const width of [initialWidth, 900, 1100, 1600]) {
      await page.setViewportSize({ width, height: 900 });
      await expect.poll(async () => page.evaluate(() => {
        const viewportWidth = document.documentElement.clientWidth;
        const elements = [
          document.documentElement,
          document.body,
          document.querySelector('[data-testid="chat-workspace-surface"]'),
          document.querySelector('[data-testid="terminal-dock"]'),
        ].filter((element): element is HTMLElement => element instanceof HTMLElement);
        return elements.map((element) => ({
          element: element.dataset.testid || element.tagName,
          overflow: Math.max(0, element.scrollWidth - element.clientWidth),
          outsideViewport: Math.max(0, element.getBoundingClientRect().right - viewportWidth),
        })).filter(({ overflow, outsideViewport }) => overflow > 1 || outsideViewport > 1);
      }), { message: `terminal and chat must fit a ${width}px viewport` }).toEqual([]);

      await expect.poll(async () => page.getByTestId('terminal-screen').evaluate((host) => {
        const screen = host.querySelector('.xterm-screen');
        if (!screen) return Number.POSITIVE_INFINITY;
        const style = getComputedStyle(host);
        const available = host.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
        return screen.getBoundingClientRect().width - available;
      }), { message: 'terminal columns must fit the available content width' }).toBeLessThanOrEqual(1);
    }

    await page.screenshot({ path: testInfo.outputPath('terminal-width.png') });
  });
}

test('opens an interactive terminal dock from the chat screen', async ({ page, context }) => {
  await page.addInitScript(() => {
    const plugin = {
      manifestVersion: 2,
      kind: 'theme-resource',
      id: 'terminal-wallpaper',
      name: 'Terminal Wallpaper',
      theme: {
        baseTheme: 'light',
        mode: 'light',
        colors: {
          surface0: 'rgba(246, 238, 232, 0.12)',
          surface1: 'rgba(255, 248, 242, 0.15)',
          textPrimary: '#251913',
          textSecondary: '#59443a',
          textTertiary: '#786056',
          accent: '#c85d2e',
        },
        effects: { surfaceOpacity: 0.37, glassBlur: 23 },
        typography: {},
        motion: {},
        brand: {},
        content: {},
        components: {},
        background: {
          kind: 'gradient',
          value: 'linear-gradient(135deg, #4f2418, #e09158)',
        },
      },
    };
    localStorage.setItem('nexa-theme-resource-plugins-v2', JSON.stringify([plugin]));
    localStorage.setItem('nexa-active-theme-v1', plugin.id);
  });
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto('/chat/conv-terminal-dock');
  const composer = page.getByTestId('chat-input');
  await expect(composer).toHaveAttribute('data-placement', 'center');

  await page.getByRole('button', { name: 'Toggle terminal' }).click();
  await expect(composer).toHaveAttribute('data-placement', 'bottom');
  await expect(page.getByTestId('terminal-dock')).toHaveAttribute('data-theme-surface', 'transparent');
  const terminalHeader = page.getByTestId('terminal-dock-header');
  const terminalScreenSurface = page.getByTestId('terminal-screen').locator('..');
  await expect(terminalHeader).toHaveAttribute('data-theme-surface', 'chrome');
  await expect(terminalHeader).toHaveCSS('backdrop-filter', 'blur(23px)');
  await expect(terminalScreenSurface).toHaveAttribute('data-theme-surface', 'overlay');
  await expect(terminalScreenSurface).toHaveCSS('backdrop-filter', 'none');
  const terminalScreenAlpha = await terminalScreenSurface.evaluate((element) => {
    const color = getComputedStyle(element).backgroundColor;
    const parts = color.match(/^rgba?\((.+)\)$/i)?.[1]
      .split(/[\s,\/]+/)
      .filter(Boolean);
    return parts && parts.length >= 4 ? Number(parts[3]) : 1;
  });
  expect(terminalScreenAlpha).toBe(1);

  await expect(page.locator('.xterm')).toBeVisible();
  await expect(page.getByText('Running')).toBeVisible();
  await expect(page.getByText('PowerShell #4242')).toBeVisible();
  await expect(page.getByTestId('terminal-agent-link')).toContainText('Terminal Dock Config');

  await page.keyboard.press('Control+KeyJ');
  await expect(page.locator('.xterm')).toHaveCount(0);
  await expect(composer).toHaveAttribute('data-placement', 'bottom');

  await page.keyboard.press('Control+KeyJ');
  await expect(page.locator('.xterm')).toBeVisible();

  await page.locator('.xterm-helper-textarea').focus();
  await page.keyboard.press('Control+C');

  await expect.poll(async () => page.evaluate(() => {
    const diagnostics = (window as unknown as {
      __terminalDiagnostics__: { writes: string[] };
    }).__terminalDiagnostics__;
    return diagnostics.writes;
  })).toContain('\u0003');

  const starts = await page.evaluate(() => {
    const diagnostics = (window as unknown as {
      __terminalDiagnostics__: { starts: Array<Record<string, unknown>> };
    }).__terminalDiagnostics__;
    return diagnostics.starts;
  });
  expect(starts).toHaveLength(1);
  expect(starts[0]).toMatchObject({
    input: {
      shell: 'default',
      conversationId: 'conv-terminal-dock',
    },
  });

  const bindings = await page.evaluate(() => {
    const diagnostics = (window as unknown as {
      __terminalDiagnostics__: { bindings: Array<Record<string, unknown>> };
    }).__terminalDiagnostics__;
    return diagnostics.bindings;
  });
  expect(bindings).toContainEqual({
    sessionId: 'terminal-session-1',
    conversationId: 'conv-terminal-dock',
  });

  const screen = page.locator('.xterm-screen');
  await page.waitForTimeout(100);
  const box = await screen.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box!.x + 4, box!.y + 38);
  await page.mouse.down();
  await page.mouse.move(box!.x + Math.min(box!.width - 8, 420), box!.y + 38, { steps: 8 });
  await page.mouse.up();

  await expect(page.getByTestId('terminal-send-selection')).toBeVisible();
  const writesBeforeCopy = await page.evaluate(() => {
    const diagnostics = (window as unknown as {
      __terminalDiagnostics__: { writes: string[] };
    }).__terminalDiagnostics__;
    return diagnostics.writes.length;
  });
  await page.keyboard.press('Control+C');
  await expect(page.getByText('Terminal selection copied')).toBeVisible();
  await expect.poll(async () => page.evaluate(() => {
    const diagnostics = (window as unknown as {
      __terminalDiagnostics__: { writes: string[] };
    }).__terminalDiagnostics__;
    return diagnostics.writes.length;
  })).toBe(writesBeforeCopy);
  await expect.poll(async () => page.evaluate(() => navigator.clipboard.readText())).toContain('ask_myself');

  await page.getByTestId('terminal-send-selection').click();
  await expect(page.getByTestId('chat-input-textarea')).toHaveValue(/<terminal_selection>/);
  await expect(page.getByTestId('chat-input-textarea')).toHaveValue(/ask_myself/);

  await page.getByRole('button', { name: 'Stop terminal' }).click();
  await expect(page.getByText('Exited')).toBeVisible();
  await page.getByRole('button', { name: 'Close terminal' }).click();
  await expect(page.getByText('Exited')).toHaveCount(0);
  await expect(composer).toHaveAttribute('data-placement', 'center');

  const afterStop = await page.evaluate(() => {
    const diagnostics = (window as unknown as {
      __terminalDiagnostics__: {
        starts: Array<Record<string, unknown>>;
        closes: string[];
      };
    }).__terminalDiagnostics__;
    return {
      startCount: diagnostics.starts.length,
      closes: diagnostics.closes,
    };
  });
  expect(afterStop).toEqual({
    startCount: 1,
    closes: ['terminal-session-1'],
  });
});
