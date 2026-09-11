import { expect, test, type Page, type WebSocketRoute } from "@playwright/test";
test.use({
  viewport: { width: 390, height: 844 },
  launchOptions: {
    args: [
      "--use-fake-ui-for-media-stream",
      "--use-fake-device-for-media-stream",
    ],
  },
});
async function fixture(page: Page, publicProbeDelayMs = 0, audioAckDelayMs = 0) {
  let lan = true;
  let away = true;
  let socket: WebSocketRoute | null = null;
  let pairCount = 0;
  let audio = 0;
  let frames = 0;
  let starts = 0;
  let stopped = 0;
  let sockets = 0;
  let uncertainStart = false;
  const launches: string[] = [];
  const actions: any[] = [];
  let preferences: any = null;
  const htmlPreviews = new Map<string, string>();
  await page.route(url => url.pathname.startsWith('/preview/'), route => route.fulfill({ contentType:'text/html', headers:{ 'content-security-policy':"sandbox allow-scripts; default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'none'" }, body:htmlPreviews.get(new URL(route.request().url()).pathname) || '' }));
  let voiceId = '';
  let voiceText = '';
  let voiceSequence = 0;
  let historyEnabled = false;
  let holdLaunch = false;
  const historyReplies: Array<() => void> = [];
  const textReplies: Array<() => void> = [];
  const launchReplies: Array<() => void> = [];
  const runEvents: any[] = [];
  let currentRunId = "run-1";
  let approvalItems: any[] = [];
  let interactionItems: any[] = [];
  const conversations = [
    { id: "chat-1", title: "Product review", model: "Qwen" },
  ];
  const manifest = {
    serverId: "desktop-1",
    endpoints: [
      { url: "https://nexa-lan.test", kind: "lan" },
      { url: "https://nexa-away.test", kind: "tunnel" },
    ],
    reconnectGraceSeconds: 30,
  };
  const live = {
    id: "live-1",
    mode: "qwenRealtime",
    model: "qwen3.5-omni-flash-realtime",
    phase: "listening",
    sampleRate: 16000,
    startedAt: new Date().toISOString(),
    sequence: 0,
    entries: [] as any[],
    metrics: {
      framesReceived: 0,
      framesSubmitted: 0,
      framesReplaced: 0,
      audioMs: 0,
      lastResponseMs: null,
      omittedEntries: 0,
    },
    error: null,
  };
  await page.addInitScript(() => {
    localStorage.setItem("nexa-locale", "en");
    const tracks: MediaStreamTrack[] = [];
    const capture = navigator.mediaDevices.getUserMedia.bind(
      navigator.mediaDevices,
    );
    navigator.mediaDevices.getUserMedia = async (constraints) => {
      const stream = await capture(constraints);
      tracks.push(...stream.getTracks());
      return stream;
    };
    Object.assign(window, { __remoteTracks: tracks });
  });
  await page.route("**/api/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (
      (url.hostname === "nexa-lan.test" && !lan) ||
      (url.hostname === "nexa-away.test" && !away)
    ) {
      await route.abort("internetdisconnected");
      return;
    }
    const headers = {
      "access-control-allow-origin": "*",
      "access-control-allow-headers": "Authorization, Content-Type",
      "access-control-allow-methods": "GET, POST, OPTIONS",
      "content-type": "application/json",
    };
    if (request.method() === "OPTIONS") {
      await route.fulfill({ status: 204, headers });
      return;
    }
    let result: unknown;
    if (url.pathname === "/api/health") {
      if (url.hostname === "nexa-away.test" && publicProbeDelayMs)
        await new Promise((resolve) => setTimeout(resolve, publicProbeDelayMs));
      result = { serverId: "desktop-1" };
    } else if (url.pathname === "/api/pair") {
      pairCount++;
      result = {
        token: "a".repeat(64),
        device: { id: "phone-1", name: "Phone" },
        manifest,
      };
    } else {
      expect(request.headers().authorization).toBe(`Bearer ${"a".repeat(64)}`);
      const { id, method, params } = request.postDataJSON();
      let value: unknown = null;
      switch (method) {
        case 'files.preview':
          actions.push({method, params});
          value = { path:params.path, displayName:'result.html', extension:'.html', kind:'text', language:'html', content:'<button id="counter">Count 0</button><script>let count=0; document.getElementById("counter").onclick=()=>document.getElementById("counter").textContent="Count "+(++count); try { parent.localStorage.getItem("nexa"); document.body.dataset.isolated="false"; } catch { document.body.dataset.isolated="true"; }</script>' };
          break;
        case 'files.data':
          actions.push({method, params});
          value = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=';
          break;
        case 'preview.html':
          actions.push({method, params}); value = '/preview/example'; htmlPreviews.set('/preview/example', params.html); break;
        case 'voice.start':
          voiceId = params.requestId;
          actions.push({method, params});
          value = { sessionId:voiceId, sampleRate:16000 };
          break;
        case 'voice.finish':
          actions.push({method, params});
          value = { text:voiceText };
          break;
        case 'voice.cancel': actions.push({method, params}); break;
        case 'connection.preferences': value = preferences; break;
        case "models.list":
          value = { connectionId: params.connectionId, discoverySucceeded: true, error: null, models: [
            { id: 'qwen-vl', name: 'Qwen VL', vision: true, reasoning: null },
            { id: 'qwen-plus', name: 'Qwen Plus', vision: true, reasoning: { mode:'optional', effortLevels:['low', 'high'] } },
          ] };
          break;
        case "connections.list":
          value = [
            { id: "qwen", name: "Qwen", model: "qwen-vl", isDefault: true },
          ];
          break;
        case "chat.list":
          value = { items: conversations, nextCursor: null };
          break;
        case "chat.read":
          if (historyEnabled && params.beforeOrder != null) {
            await new Promise<void>((resolve) => historyReplies.push(resolve));
            value = {
              messages: [
                {
                  id: `older-${params.beforeOrder}`,
                  role: "assistant",
                  content: `Earlier ${params.beforeOrder}`,
                  totalChars: 10,
                  sortOrder: params.beforeOrder - 1,
                },
              ],
              beforeOrder: params.beforeOrder - 5,
            };
            break;
          }
          value = {
            messages: [
              {
                id: params.conversationId === "chat-2" ? "m2" : "m1",
                role: "assistant",
                content: historyEnabled
                  ? params.conversationId === "chat-2"
                    ? "Second conversation"
                    : "First chunk"
                  : "The agent is ready on your computer.",
                totalChars: historyEnabled
                  ? params.conversationId === "chat-2"
                    ? 19
                    : 18
                  : 36,
                sortOrder: 0,
              },
            ],
            beforeOrder:
              historyEnabled && params.conversationId === "chat-1" ? 10 : null,
          };
          break;
        case "chat.message":
          await new Promise<void>((resolve) => textReplies.push(resolve));
          value = { content: " + tail", totalChars: 18 };
          break;
        case "chat.create":
          value = conversations[0];
          break;
        case "chat.resume":
          value = {
            run: runEvents.length
              ? { id: currentRunId, status: "running" }
              : null,
            events: runEvents.filter(
              (event) =>
                event.eventSeq >
                (params.runId === currentRunId ? params.afterSequence || 0 : 0),
            ),
            hasMore: false,
          };
          break;
        case "chat.start":
          actions.push({ method, params });
          launches.push(params.idempotencyKey);
          if (holdLaunch)
            await new Promise<void>((resolve) => launchReplies.push(resolve));
          if (uncertainStart) {
            uncertainStart = false;
            lan = false;
            socket?.close({ code: 1001 });
            await route.abort("connectionreset");
            return;
          }
          value = { runId: "run-1" };
          break;
        case "approvals.list":
          value = approvalItems;
          break;
        case "interactions.list":
          value = interactionItems;
          break;
        case "approvals.respond":
          actions.push({ method, params });
          approvalItems = [];
          break;
        case "interactions.respond":
          actions.push({ method, params });
          interactionItems = [];
          break;
        case "chat.stop":
          actions.push({ method, params });
          break;
        case "live.connections":
          value = [
            {
              id: "qwen",
              name: "Qwen",
              model: "qwen-vl",
              isDefault: true,
              nativeProtocols: ["qwenRealtime"],
              vision: "supported",
            },
          ];
          break;
        case "live.list":
          value = stopped
            ? [{ id: live.id, startedAt: live.startedAt, model: live.model }]
            : [];
          break;
        case "live.start":
          actions.push({ method, params });
          starts++;
          value = live;
          break;
        case "live.snapshot":
          value = live;
          break;
        case "live.frame":
          frames++;
          live.metrics.framesSubmitted = frames;
          live.entries = [
            {
              id: "observation",
              role: "assistant",
              text: "The prototype is visible. Review is scheduled for Friday.",
              atMs: 1000,
              complete: true,
            },
          ];
          socket?.send(
            JSON.stringify({
              event: "live:event",
              payload: {
                type: "entry",
                entry: live.entries[0],
                sessionId: live.id,
                sequence: ++live.sequence,
              },
            }),
          );
          break;
        case "live.stop":
          stopped++;
          live.phase = "stopped";
          value = live;
          break;
        case "live.summarize":
          actions.push({ method, params });
          value = {
            snapshot: live,
            summary:
              "Review the prototype on Friday; owner is still unconfirmed.",
          };
          break;
        case "live.load":
          value = { snapshot: live, summary: null };
          break;
      }
      result = { id, result: value };
    }
    await route.fulfill({ headers, body: JSON.stringify(result) });
  });
  await page.routeWebSocket("**/api/events", (ws) => {
    sockets++;
    socket = ws;
    ws.onMessage((message) => {
      if (message === "ping") {
        ws.send(JSON.stringify({ event: "connection:heartbeat", payload: {} }));
        return;
      }
      const input = JSON.parse(String(message));
      if (input.token)
        ws.send(
          JSON.stringify({
            event: "connection:ready",
            payload: { device: { id: "phone-1" }, manifest },
          }),
        );
      else if (input.method === "live.audio" || input.method === 'voice.audio') {
        audio++;
        const bytes = Buffer.from(input.params.data, "base64").length;
        // Dictation flushes a final partial PCM packet before voice.finish.
        expect(bytes).toBeGreaterThan(input.method === 'voice.audio' ? 0 : 1000);
        expect(bytes % 2).toBe(0);
        setTimeout(() => ws.send(
          JSON.stringify({
            event: "connection:ack",
            payload: { id: input.id },
          }),
        ), audioAckDelayMs);
      }
    });
  });
  const emit = (kind: string, payload: unknown, deliver = true) => {
    const event = {
      version: 2,
      runId: currentRunId,
      eventSeq: runEvents.length + 1,
      kind,
      label: kind === "approvalRequested" ? "Review this action" : "Working",
      visibility: "user",
      payload,
    };
    runEvents.push(event);
    if (deliver)
      socket?.send(
        JSON.stringify({
          event: "agent://run-event",
          payload: { conversationId: "chat-1", runEvent: event },
        }),
      );
  };
  return {
    backupRoute: () => { manifest.endpoints.push({url:'https://nexa-backup.test',kind:'tunnel'}); },
    audioDelay: (delay: number) => { audioAckDelayMs = delay; },
    leaveLan: () => { lan = false; socket?.close({code:1001}); },
    connectedUrl: () => socket?.url(),
    dictation: (text: string) => {
      voiceText = text;
      socket?.send(JSON.stringify({event:'voice:event',payload:{sessionId:voiceId,sequence:++voiceSequence,kind:'interim',text}}));
    },
    preferences: (value: any) => { preferences = value; },
    counts: () => ({
      pairCount,
      audio,
      frames,
      starts,
      stopped,
      sockets,
      launches,
      actions,
    }),
    emit,
    history: () => {
      historyEnabled = true;
      conversations.push({ id: "chat-2", title: "Second chat", model: "Qwen" });
    },
    pendingPages: () => ({
      history: historyReplies.length,
      text: textReplies.length,
      launches: launchReplies.length,
    }),
    releaseHistory: () => historyReplies.shift()?.(),
    releaseText: () => textReplies.shift()?.(),
    holdLaunch: () => {
      holdLaunch = true;
    },
    releaseLaunch: () => launchReplies.shift()?.(),
    nextRun: () => {
      currentRunId = "run-2";
      runEvents.length = 0;
    },
    requests: (questionType = "single_choice") => {
      approvalItems = [
        {
          runId: "run-1",
          request: {
            id: "approval-1",
            toolName: "write_file",
            reason: "Update the reviewed document",
            targetValue: "report.md",
            argumentsPreview: '{"path":"report.md"}',
          },
        },
      ];
      interactionItems = [
        {
          interactionId: "question-1",
          title: "Choose the release day",
          resumeToken: "resume-1",
          questions: [
            {
              id: "day",
              question: "When should this ship?",
              type: questionType,
              options: [{ label: "Friday" }, { label: "Monday" }],
            },
          ],
        },
      ];
    },
    uncertain: () => {
      uncertainStart = true;
    },
    disconnect: () => {
      lan = false;
      away = false;
      socket?.close({ code: 1001 });
    },
    restoreAway: () => {
      away = true;
    },
    restoreLan: () => {
      lan = true;
    },
    revoke: () => socket?.close({ code: 1008 }),
  };
}
test("QR pairs once and retries an uncertain chat launch with the same ID after leaving LAN", async ({
  page,
}) => {
  const app = await fixture(page);
  await page.goto("/phone.html#pair=123456&server=desktop-1");
  await expect(
    page.getByRole("button", { name: "Local network", exact: true }),
  ).toBeVisible();
  expect(page.url()).not.toContain("#");
  expect(app.counts().pairCount).toBe(1);
  await page.getByLabel("Conversation", { exact: true }).selectOption("chat-1");
  await page
    .getByLabel("Message", { exact: true })
    .fill("Summarize the open work.");
  app.uncertain();
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Public connection", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel("Message", { exact: true })).toHaveValue("");
  expect(app.counts().launches).toHaveLength(2);
  expect(new Set(app.counts().launches).size).toBe(1);
  await page.screenshot({
    path: ".artifacts/remote-phone-chat.png",
    fullPage: true,
  });
  await page.reload();
  await expect(page.getByTestId("remote-chat")).toBeVisible();
  expect(app.counts().pairCount).toBe(1);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});

test("phone resumes the latest desktop run after missing its launch while offline", async ({
  page,
}) => {
  const app = await fixture(page);
  await page.goto("/phone.html#pair=123456&server=desktop-1");
  await page.getByLabel("Conversation", { exact: true }).selectOption("chat-1");
  app.emit("outputDelta", {
    blockId: "a",
    channel: "answer",
    offset: 0,
    delta: "Old run answer",
  });
  await expect(page.getByText("Old run answer", { exact: true })).toBeVisible();
  app.disconnect();
  app.nextRun();
  app.emit(
    "outputDelta",
    { blockId: "b", channel: "answer", offset: 0, delta: "New desktop run" },
    false,
  );
  app.restoreAway();
  await page.evaluate(() => window.dispatchEvent(new Event("online")));
  await expect(
    page.getByText("New desktop run", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("Old run answer", { exact: true })).toHaveCount(
    0,
  );
});
test('automatic route selection reaches a fast backup without waiting for a stalled public route', async ({ page }) => {
  const app = await fixture(page, 6000);
  app.backupRoute();
  await page.goto('/phone.html#pair=123456&server=desktop-1');
  await expect(page.getByRole('button',{name:'Local network',exact:true})).toBeVisible();
  app.leaveLan();
  await page.evaluate(() => window.dispatchEvent(new Event('online')));
  await expect.poll(() => app.connectedUrl(), {timeout:3000}).toBe('wss://nexa-backup.test/api/events');
  await expect(page.getByRole('button',{name:'Public connection',exact:true})).toBeVisible();
});

test('phone dictation edits the composer live and retains manual corrections through finalization', async ({ page, context }) => {
  await context.grantPermissions(['microphone']);
  const app = await fixture(page, 0, 350);
  await page.goto('/phone.html#pair=123456&server=desktop-1');
  const draft = page.getByLabel('Message', {exact:true});
  await draft.fill('写下：');
  await page.getByRole('button', {name:'Voice input',exact:true}).click();
  await expect.poll(() => app.counts().audio).toBeGreaterThan(3);
  app.dictation('今天天气很好');
  await expect(draft).toHaveValue('写下： 今天天气很好');
  await draft.fill('写下： 明天天气很好');
  app.dictation('今天天气很好啊');
  await expect(draft).toHaveValue('写下： 明天天气很好啊');
  await page.getByRole('button', {name:'Finish dictation',exact:true}).click();
  await expect.poll(() => app.counts().actions.some(item => item.method === 'voice.finish')).toBe(true);
  await expect(page.getByRole('button', {name:'Voice input',exact:true})).toBeEnabled();
  await expect(draft).toHaveValue('写下： 明天天气很好啊');
  await expect.poll(() => page.evaluate(() => (window as any).__remoteTracks.every((track: MediaStreamTrack) => track.readyState === 'ended'))).toBe(true);
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('phone renders Markdown, code, formulas and diagrams from the desktop run', async ({ page }, testInfo) => {
  const app = await fixture(page);
  await page.goto('/phone.html#pair=123456&server=desktop-1');
  await page.getByLabel('Conversation',{exact:true}).selectOption('chat-1');
  const content = '# Remote result\n\n| Item | Count |\n|---|---|\n| Done | 3 |\n\n```typescript\nconst answer = 42;\n```\n\n$E = mc^2$\n\n```mermaid\ngraph LR\n A[Phone] --> B[Desktop]\n```';
  app.emit('outputSnapshot', {blockId:'answer',channel:'answer',text:content});
  await expect(page.getByRole('heading',{name:'Remote result'})).toBeVisible();
  await expect(page.getByRole('table')).toHaveCount(1);
  await expect(page.locator('.katex').first()).toBeVisible();
  // Streaming diagrams intentionally defer execution until the model finishes.
  await expect(page.locator('[data-testid="mermaid-surface"]')).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path:testInfo.outputPath('phone-chat.png'), fullPage:true });
});

test('phone sends attachments and execution choices with an idempotent retry', async ({ page }) => {
  const app = await fixture(page);
  await page.goto('/phone.html#pair=123456&server=desktop-1');
  await page.getByLabel('Attach files', {exact:true}).setInputFiles({ name:'brief.txt', mimeType:'text/plain', buffer:Buffer.from('Review the product brief') });
  await expect(page.getByText('brief.txt', {exact:true})).toBeVisible();
  await page.getByLabel('Execution mode', {exact:true}).selectOption('plan');
  await page.getByLabel('Nexus', {exact:true}).check();
  app.uncertain();
  await page.getByRole('button', {name:'Send',exact:true}).click();
  await expect.poll(() => app.counts().actions.filter(item => item.method === 'chat.start').length).toBe(2);
  const requests = app.counts().actions.filter(item => item.method === 'chat.start').map(item => item.params);
  expect(requests[0]).toEqual(requests[1]);
  expect(requests[0]).toMatchObject({ executionMode:'plan', powerMode:'nexus', attachments:[{originalName:'brief.txt',base64Data:Buffer.from('Review the product brief').toString('base64')}] });
  await expect(page.getByText('brief.txt', {exact:true})).toHaveCount(0);
});

test('phone previews local images and interactive HTML without exposing the paired page', async ({ page }, testInfo) => {
  const app = await fixture(page);
  await page.goto('/phone.html#pair=123456&server=desktop-1');
  await page.getByLabel('Conversation', {exact:true}).selectOption('chat-1');
  app.emit('outputSnapshot', {blockId:'answer',channel:'answer',text:'![Local result](C:/reports/result.png)\n\n[Open report](C:/reports/result.html)'});
  await expect.poll(() => app.counts().actions.some(item => item.method === 'files.data')).toBe(true);
  await page.getByRole('button', {name:'Open report',exact:true}).click();
  const frame = page.frameLocator('iframe[title="result.html"]');
  await expect(frame.getByRole('button', {name:'Count 0',exact:true})).toBeVisible();
  await frame.getByRole('button', {name:'Count 0',exact:true}).click();
  await expect(frame.getByRole('button', {name:'Count 1',exact:true})).toBeVisible();
  await expect(frame.locator('body')).toHaveAttribute('data-isolated','true');
  expect(await page.locator('iframe').getAttribute('sandbox')).toBe('allow-scripts');
  await expect(page.getByRole('dialog')).toHaveCSS('position','fixed');
  await page.screenshot({ path:testInfo.outputPath('phone-html-preview.png') });
  await page.getByRole('button', {name:'Close',exact:true}).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('phone follows desktop theme and language with persistent independent overrides', async ({ page }) => {
  const app = await fixture(page);
  app.preferences({ locale:'zh-CN', appearance:{ version:2, initialized:true, revision:1, activeThemeId:'dream', plugins:[] } });
  await page.goto('/phone.html#pair=123456&server=desktop-1');
  await expect(page.locator('html')).toHaveClass(/theme-dream/);
  await expect(page.locator('html')).toHaveAttribute('lang', 'zh-CN');
  await page.getByRole('button', { name:'局域网', exact:true }).click();
  await page.getByLabel('皮肤', {exact:true}).selectOption('light');
  await page.getByLabel('语言', {exact:true}).selectOption('en');
  await expect(page.locator('html')).toHaveClass(/theme-light/);
  await page.reload();
  await expect(page.locator('html')).toHaveClass(/theme-light/);
  await expect(page.locator('html')).toHaveAttribute('lang','en');
});

test("remote Chat and Live choose provider models independently of saved desktop defaults", async ({ page, context }) => {
  await context.grantPermissions(["microphone"]);
  const app = await fixture(page);
  await page.goto('/phone.html#pair=123456&server=desktop-1');
  await page.getByLabel('Model connection', { exact:true }).selectOption('qwen-plus');
  await page.getByLabel('Model connection · Reasoning Effort', { exact:true }).selectOption('high');
  await page.getByLabel('Message', { exact:true }).fill('Use the newly selected model');
  await page.getByRole('button', { name:'Send', exact:true }).click();
  await expect.poll(() => app.counts().actions.find(item => item.method === 'chat.start')?.params.modelSelection).toEqual({ model:'qwen-plus', reasoningEnabled:true, reasoningEffort:'high', thinkingBudget:null });
  await page.getByRole('button', { name:'Live', exact:true }).click();
  await page.getByLabel('Observation model', { exact:true }).selectOption('qwen-plus');
  await page.getByLabel('Summary model', { exact:true }).selectOption('qwen-vl');
  await page.getByRole('button', { name:'Start Live', exact:true }).click();
  await expect.poll(() => app.counts().actions.find(item => item.method === 'live.start')?.params.request.modelSelection?.model).toBe('qwen-plus');
  await page.getByRole('button', { name:'Stop capture', exact:true }).click();
});

test("remote Live sustains microphone capture with 350 ms acknowledgement latency", async ({ page, context }) => {
  await context.grantPermissions(["microphone"]);
  const app = await fixture(page, 0, 350);
  await page.goto("/phone.html#pair=123456&server=desktop-1");
  await page.getByRole("button", { name: "Live", exact: true }).click();
  await page.getByRole("button", { name: "Start Live", exact: true }).click();
  await expect.poll(() => app.counts().audio, { timeout: 7000 }).toBeGreaterThan(35);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Stop capture", exact: true })).toBeVisible();
  expect(app.counts().starts).toBe(1);
  await page.getByRole("button", { name: "Stop capture", exact: true }).click();
});

test('remote Live reconnects a stalled audio route without a terminal backpressure error', async ({ page, context }) => {
  await context.grantPermissions(['microphone']);
  const app = await fixture(page, 0, 6000);
  await page.goto('/phone.html#pair=123456&server=desktop-1');
  await page.getByRole('button', {name:'Live',exact:true}).click();
  await page.getByRole('button', {name:'Start Live',exact:true}).click();
  await expect.poll(() => app.counts().sockets, {timeout:8000}).toBeGreaterThan(1);
  app.audioDelay(0);
  const audio = app.counts().audio;
  await expect.poll(() => app.counts().audio).toBeGreaterThan(audio + 5);
  expect(app.counts().starts).toBe(1); expect(app.counts().stopped).toBe(0);
  await expect(page.getByRole('alert')).toHaveCount(0);
  await page.getByRole('button', {name:'Stop capture',exact:true}).click();
});

test("phone microphone and camera pause across a route change then resume the same Live session", async ({
  page,
  context,
}) => {
  await context.grantPermissions(["microphone", "camera"]);
  const app = await fixture(page);
  await page.goto("/phone.html#pair=123456&server=desktop-1");
  await page.getByRole("button", { name: "Live", exact: true }).click();
  await page
    .getByRole("combobox", { name: "Analysis mode", exact: true })
    .selectOption("qwenRealtime");
  await page
    .getByLabel("Qwen workspace endpoint", { exact: true })
    .fill("wss://test.cn-beijing.maas.aliyuncs.com/api-ws/v1/realtime");
  await page.getByLabel("Visual input", { exact: true }).selectOption("camera");
  await page.getByRole("button", { name: "Start Live", exact: true }).click();
  await expect.poll(() => app.counts().audio).toBeGreaterThan(3);
  await expect.poll(() => app.counts().frames).toBeGreaterThan(0);
  app.disconnect();
  await expect(page.getByText(/Finding a reachable connection/)).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__remoteTracks
          .filter((track: MediaStreamTrack) => track.kind === "video")
          .every((track: MediaStreamTrack) => !track.enabled),
      ),
    )
    .toBe(true);
  const pausedAudio = app.counts().audio;
  await page.waitForTimeout(400);
  expect(app.counts().audio).toBe(pausedAudio);
  app.restoreAway();
  await expect(
    page.getByRole("button", { name: "Public connection", exact: true }),
  ).toBeVisible();
  await expect.poll(() => app.counts().audio).toBeGreaterThan(pausedAudio + 3);
  expect(app.counts().starts).toBe(1);
  app.restoreLan();
  await page.evaluate(() => window.dispatchEvent(new Event("online")));
  await expect(
    page.getByRole("button", { name: "Local network", exact: true }),
  ).toBeVisible();
  const returnedAudio = app.counts().audio;
  await expect
    .poll(() => app.counts().audio)
    .toBeGreaterThan(returnedAudio + 3);
  expect(app.counts().starts).toBe(1);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({
    path: ".artifacts/remote-phone-live.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "Stop capture", exact: true }).click();
  await expect.poll(() => app.counts().stopped).toBe(1);
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__remoteTracks.every(
          (track: MediaStreamTrack) => track.readyState === "ended",
        ),
      ),
    )
    .toBe(true);
  await page
    .getByRole("button", { name: "Create summary", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Continue in chat", exact: true })
    .click();
  await expect(page.getByLabel("Message", { exact: true })).toHaveValue(
    /owner is still unconfirmed/,
  );
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});
test("revoking the phone releases capture and exposes the pairing recovery action", async ({
  page,
  context,
}) => {
  await context.grantPermissions(["microphone", "camera"]);
  const app = await fixture(page);
  await page.goto("/phone.html#pair=123456&server=desktop-1");
  await page.getByRole("button", { name: "Live", exact: true }).click();
  await page.getByRole("button", { name: "Start Live", exact: true }).click();
  await expect.poll(() => app.counts().audio).toBeGreaterThan(1);
  app.revoke();
  await expect(
    page.getByRole("button", { name: "Access revoked", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__remoteTracks.every(
          (track: MediaStreamTrack) => track.readyState === "ended",
        ),
      ),
    )
    .toBe(true);
  await page
    .getByRole("button", { name: "Access revoked", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Forget this connection", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Your agent, within reach" }),
  ).toBeVisible();
});

test("phone replays missing UTF-8 deltas and forwards explicit approvals, answers, and stop", async ({
  page,
}) => {
  const app = await fixture(page);
  await page.goto("/phone.html#pair=123456&server=desktop-1");
  await page.getByLabel("Conversation", { exact: true }).selectOption("chat-1");
  await expect(
    page.getByText("The agent is ready on your computer."),
  ).toBeVisible();
  app.emit("outputDelta", {
    blockId: "a",
    channel: "answer",
    offset: 0,
    delta: "你",
  });
  app.emit(
    "outputDelta",
    { blockId: "a", channel: "answer", offset: 3, delta: "-" },
    false,
  );
  app.emit("outputDelta", {
    blockId: "a",
    channel: "answer",
    offset: 4,
    delta: "好🙂",
  });
  await expect(page.getByText("你-好🙂", { exact: true })).toBeVisible();
  app.requests();
  app.emit("approvalRequested", {});
  await page.getByRole("button", { name: "Allow once", exact: true }).click();
  await expect
    .poll(
      () =>
        app
          .counts()
          .actions.find((action) => action.method === "approvals.respond")
          ?.params,
    )
    .toEqual({ requestId: "approval-1", decision: "allow_once" });
  await page.getByRole("radio", { name: "Friday", exact: true }).check();
  await page
    .getByRole("button", { name: "Submit answer", exact: true })
    .click();
  await expect
    .poll(
      () =>
        app
          .counts()
          .actions.find((action) => action.method === "interactions.respond")
          ?.params.input,
    )
    .toEqual({
      interactionId: "question-1",
      resumeToken: "resume-1",
      answers: { day: ["Friday"] },
    });
  await page.getByRole("button", { name: "Stop", exact: true }).click();
  await expect
    .poll(() =>
      app.counts().actions.some((action) => action.method === "chat.stop"),
    )
    .toBe(true);
});

test("phone custom multi-choice answers preserve checked options through edits and clearing", async ({
  page,
}) => {
  const app = await fixture(page);
  await page.goto("/phone.html#pair=123456&server=desktop-1");
  await page.getByLabel("Conversation", { exact: true }).selectOption("chat-1");
  await expect(
    page.getByText("The agent is ready on your computer."),
  ).toBeVisible();
  app.requests("multi_choice");
  app.emit("approvalRequested", {});
  const friday = page.getByRole("checkbox", { name: "Friday", exact: true });
  const monday = page.getByRole("checkbox", { name: "Monday", exact: true });
  const custom = page.getByLabel("When should this ship?", { exact: true });
  await friday.check();
  await monday.check();
  await custom.fill("After the review");
  await expect(friday).toBeChecked();
  await expect(monday).toBeChecked();
  await custom.fill("");
  await expect(friday).toBeChecked();
  await expect(monday).toBeChecked();
  await custom.fill("Tuesday morning");
  await monday.uncheck();
  await expect(custom).toHaveValue("Tuesday morning");
  await page.getByRole("button", { name: "Submit answer", exact: true }).click();
  await expect
    .poll(
      () =>
        app.counts().actions.find(
          (action) => action.method === "interactions.respond",
        )?.params.input,
    )
    .toEqual({
      interactionId: "question-1",
      resumeToken: "resume-1",
      answers: { day: ["Friday", "Tuesday morning"] },
    });
});

test("a slower public health probe still connects when the phone is away", async ({
  page,
}) => {
  const app = await fixture(page, 2500);
  app.disconnect();
  app.restoreAway();
  await page.goto("/phone.html#pair=123456&server=desktop-1");
  await expect(
    page.getByRole("button", { name: "Public connection", exact: true }),
  ).toBeVisible({ timeout: 10_000 });
  expect(app.counts().pairCount).toBe(1);
});

test("phone history paging rejects duplicate and cross-conversation responses", async ({
  page,
}) => {
  const app = await fixture(page);
  app.history();
  await page.goto("/phone.html#pair=123456&server=desktop-1");
  await page.getByLabel("Conversation", { exact: true }).selectOption("chat-1");
  await page.getByRole("button", { name: "Read more", exact: true }).click();
  await page.getByRole("button", { name: "Read more", exact: true }).click();
  await expect.poll(() => app.pendingPages().text).toBe(2);
  app.releaseText();
  app.releaseText();
  await expect(
    page.getByText("First chunk + tail", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Earlier messages", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Earlier messages", exact: true })
    .click();
  await expect.poll(() => app.pendingPages().history).toBe(2);
  app.releaseHistory();
  app.releaseHistory();
  await expect(page.getByText("Earlier 10", { exact: true })).toHaveCount(1);
  await page
    .getByRole("button", { name: "Earlier messages", exact: true })
    .click();
  await expect.poll(() => app.pendingPages().history).toBe(1);
  await page.getByLabel("Conversation", { exact: true }).selectOption("chat-2");
  await expect(
    page.getByText("Second conversation", { exact: true }),
  ).toBeVisible();
  app.releaseHistory();
  await page.waitForTimeout(200);
  await expect(page.getByText("Earlier 5", { exact: true })).toHaveCount(0);
});

test("an old chat launch cannot clear the new conversation draft", async ({
  page,
}) => {
  const app = await fixture(page);
  app.history();
  app.holdLaunch();
  await page.goto("/phone.html#pair=123456&server=desktop-1");
  await page.getByLabel("Conversation", { exact: true }).selectOption("chat-1");
  await page
    .getByLabel("Message", { exact: true })
    .fill("Start the first task");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect.poll(() => app.pendingPages().launches).toBe(1);
  await page.getByLabel("Conversation", { exact: true }).selectOption("chat-2");
  await expect(
    page.getByText("Second conversation", { exact: true }),
  ).toBeVisible();
  await page.getByLabel("Message", { exact: true }).fill("Keep this new draft");
  app.releaseLaunch();
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeEnabled();
  await expect(page.getByLabel("Message", { exact: true })).toHaveValue(
    "Keep this new draft",
  );
  await expect(
    page.getByRole("button", { name: "Stop", exact: true }),
  ).toHaveCount(0);
});
