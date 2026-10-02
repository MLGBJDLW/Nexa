import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale','en');
    const now = '2026-10-02T00:00:00Z';
    const conversation = { id:'history',title:'Paged history',provider:'open_ai',model:'gpt-4.1',systemPrompt:'',createdAt:now,updatedAt:now,initialAutoTitlePending:false };
    const config = { id:'config',name:'Test model',provider:'open_ai',apiKey:'',baseUrl:null,model:'gpt-4.1',temperature:0.3,maxTokens:4096,contextWindow:100000,isDefault:true,createdAt:now,updatedAt:now };
    const requests: Record<string,unknown>[] = [], detailRequests: string[] = [], legacyReads: string[] = [], browserOpens: unknown[] = [];
    window.addEventListener('nexa:open-browser-workspace', event => {
      browserOpens.push((event as CustomEvent).detail);
      event.preventDefault();
      event.stopImmediatePropagation();
    });
    const callbacks = new Map<number,(event:unknown)=>void>(), listeners = new Map<number,{event:string;handler:number}>();
    let sequence=0;
    const cursor = (number:number) => ({ sortOrder:number*10,messageId:`u${number}` });
    const privateReasoning = 'Private intermediate reasoning from a legacy interrupted turn.';
    const isReasoningOnly = (number:number) => number === 10000 && localStorage.getItem('timeline-reasoning-only') === '1';
    const message = (number:number,role:'user'|'assistant',details=false) => ({ id:`${role==='user'?'u':'a'}${number}`,conversationId:'history',role,content:role==='user'?`Request ${number}`:isReasoningOnly(number)?details?privateReasoning:'':`Answer ${number}. This reply belongs to the selected durable turn.`,toolCallId:null,toolCalls:[],artifacts:role==='assistant'&&isReasoningOnly(number)&&!details?{kind:'generatedImage',dataUrl:'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6zVYAAAAASUVORK5CYII=',displayReasoningOnly:true}:null,thinking:role==='assistant'&&isReasoningOnly(number)&&details?privateReasoning:null,tokenCount:1,createdAt:now,sortOrder:number*10+(role==='user'?0:2) });
    const turn = (number:number) => ({ id:`t${number}`,conversationId:'history',userMessageId:`u${number}`,assistantMessageId:`a${number}`,status:'completed',trace:null,createdAt:now,updatedAt:now,finishedAt:now });
    const mcpResult = { kind:'mcpToolResult',version:1,toolIdentity:{connectorId:'fixture-connector',toolName:'inspect_report',trustConfigDigest:'fixture'},contentBlocks:[
      {type:'text',text:'MCP evidence survives reopening.'},
      {type:'image',mimeType:'image/png',data:'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6zVYAAAAASUVORK5CYII='},
      {type:'resource_link',uri:'https://example.org/report',name:'Evidence report'},
      {type:'resource',resource:{uri:'fixture://notes',mimeType:'text/plain',text:'Embedded report note'}},
    ],structuredContent:{verified:true,rows:42},notices:[] };
    const invoke = async (cmd:string,args:Record<string,any>={}) => {
      switch(cmd) {
        case 'plugin:event|listen': { const id=++sequence;listeners.set(id,{event:args.event,handler:args.handler});return id; }
        case 'plugin:event|unlisten': listeners.delete(args.eventId);return null;
        case 'get_wizard_state_cmd':return {completed:true,language:'en'};
        case 'list_agent_configs_cmd':return [config];
        case 'get_model_context_window':return 100000;
        case 'list_conversations_cmd':return [conversation];
        case 'get_conversation_timeline_page_cmd': {
          requests.push(args);
          const start = args.anchorMessageId ? Number(String(args.anchorMessageId).slice(1)) : args.after ? Number(args.after.messageId.slice(1)) : args.before ? Math.max(1,Number(args.before.messageId.slice(1))-20) : 9981;
          const end = Math.min(10000,args.before ? Number(args.before.messageId.slice(1))-1 : start+19);
          const numbers=Array.from({length:end-start+1},(_,index)=>start+index);
          return {conversation,messages:numbers.flatMap(number=>[message(number,'user'),message(number,'assistant')]),turns:numbers.map(turn),entries:numbers.map(number=>({anchor:cursor(number),turnId:`t${number}`,hasDetails:true,detailRevision:'1'})),taskRuns:[],range:{from:cursor(start),before:end<10000?cursor(end+1):null},oldestCursor:cursor(start),newestCursor:cursor(end),hasMoreBefore:start>1,hasMoreAfter:end<10000};
        }
        case 'get_conversation_timeline_details_cmd': {
          detailRequests.push(args.anchorMessageId);
          const number=Number(args.anchorMessageId.slice(1));
          if (isReasoningOnly(number)) return {anchorId:args.anchorMessageId,messages:[message(number,'user'),message(number,'assistant',true)],turns:[{...turn(number),trace:{kind:'turnTrace',items:[{kind:'thinking',text:privateReasoning}]}}],range:{from:cursor(number),before:null},detailRevision:'1'};
          return {anchorId:args.anchorMessageId,messages:[message(number,'user'),message(number,'assistant')],turns:[{...turn(number),trace:{kind:'turnTrace',routeKind:'InteractionOperation',items:[{kind:'tool',toolCall:{callId:`mcp-${number}`,toolName:'mcp__inspect_report__opaquealias',arguments:'{}',status:'done',argsStatus:'done',argsBytes:2,content:'MCP evidence',artifacts:mcpResult}}]}}],range:{from:cursor(number),before:number<10000?cursor(number+1):null},detailRevision:'1'};
        }
        case 'get_conversation_cmd': case 'get_conversation_turns_cmd': case 'get_agent_task_runs_cmd':legacyReads.push(cmd);throw new Error('Unbounded history read');
        case 'list_pending_tool_approvals_cmd':case 'list_interaction_requests_cmd':case 'list_sources':case 'list_projects_cmd':case 'list_personas_cmd':case 'list_checkpoints_cmd':case 'list_skills_cmd':case 'list_mcp_servers_cmd':case 'list_user_memories_cmd':case 'get_conversation_sources_cmd':case 'get_conversation_file_changes_cmd':return [];
        default:return null;
      }
    };
    Object.assign(window,{__TIMELINE_TEST__:{requests,detailRequests,legacyReads,browserOpens},__TAURI_INTERNALS__:{invoke,metadata:{currentWindow:{label:'main'}},convertFileSrc:(path:string)=>path,transformCallback:(callback:(event:unknown)=>void)=>{const id=++sequence;callbacks.set(id,callback);return id;},unregisterCallback:(id:number)=>callbacks.delete(id)},__TAURI_EVENT_PLUGIN_INTERNALS__:{unregisterListener:(_event:string,id:number)=>listeners.delete(id)}});
  });
});

test('10k history opens a bounded tail and prepending retains the visible turn',async({page})=>{
  await page.goto('/chat/history');
  await expect(page.locator('[data-timeline-key="turn-t10000"]').getByText('Request 10000',{exact:true})).toBeVisible();
  expect(await page.evaluate(()=>(window as any).__TIMELINE_TEST__.detailRequests)).toEqual([]);
  const scroller=page.locator('[data-chat-scroll-root]');
  await scroller.evaluate(element=>{element.scrollTop=0;element.dispatchEvent(new Event('scroll'));});
  await expect(page.getByTestId('chat-load-older')).toBeVisible();
  const anchor=page.locator('[data-timeline-key="turn-t9981"]');
  await expect(anchor).toBeVisible();
  const top=(await anchor.boundingBox())!.y;
  await page.getByTestId('chat-load-older').click();
  await expect.poll(async()=>page.evaluate(()=>(window as any).__TIMELINE_TEST__.requests.filter((request:any)=>request.before).length)).toBe(1);
  await expect(anchor).toBeVisible();
  await expect.poll(async()=>Math.abs((await anchor.boundingBox())!.y-top)).toBeLessThan(3);
  const state=await page.evaluate(()=>(window as any).__TIMELINE_TEST__);
  expect(state.requests.find((request:any)=>request.before).before.messageId).toBe('u9981');
  expect(state.legacyReads).toEqual([]);
  expect(await page.locator('[data-timeline-key]').count()).toBeLessThan(35);
});

test('deep links read the target page and return to the latest tail',async({page})=>{
  await page.goto('/chat/history?message=a50');
  await expect(page.locator('[data-timeline-key="turn-t50"]').getByText('Request 50',{exact:true})).toBeVisible();
  await expect(page.getByTestId('chat-load-latest')).toBeVisible();
  expect(await page.evaluate(()=>(window as any).__TIMELINE_TEST__.requests[0].anchorMessageId)).toBe('a50');
  await page.getByTestId('chat-load-latest').click();
  await expect(page.locator('[data-timeline-key="turn-t10000"]').getByText('Request 10000',{exact:true})).toBeVisible();
  expect(await page.evaluate(()=>(window as any).__TIMELINE_TEST__.legacyReads)).toEqual([]);
});

test('lazy history shows typed MCP media, links and structured content after reopening',async({page})=>{
  await page.goto('/chat/history');
  const expand=async()=>{
    await page.getByTestId('deferred-activity-u10000').getByRole('button').click();
    const card=page.getByTestId('tool-call-card').filter({hasText:'inspect_report'});
    await expect(card).toBeVisible();
    await card.click();
    const result=page.getByTestId('mcp-result');
    await expect(result).toContainText('MCP evidence survives reopening.');
    await expect(result.getByRole('link',{name:'Evidence report'})).toHaveAttribute('href','https://example.org/report');
    expect(await page.evaluate(()=>(window as any).__TIMELINE_TEST__.browserOpens)).toEqual([]);
    await result.getByRole('link',{name:'Evidence report'}).click();
    expect(await page.evaluate(()=>(window as any).__TIMELINE_TEST__.browserOpens)).toEqual([{url:'https://example.org/report',title:'Evidence report'}]);
    await expect(page).toHaveURL(/\/chat\/history$/);
    await expect(result).toContainText('Embedded report note');
    await result.getByText('Structured result',{exact:true}).click();
    await expect(result).toContainText('"rows": 42');
    await result.getByRole('button',{name:'Preview: Tool result image',exact:true}).click();
    await expect(page.getByTestId('image-lightbox')).toBeVisible();
    await page.keyboard.press('Escape');
  };
  await expand();
  expect(await page.evaluate(()=>(window as any).__TIMELINE_TEST__.detailRequests)).toEqual(['u10000']);
  await page.reload();
  await expand();
  expect(await page.evaluate(()=>(window as any).__TIMELINE_TEST__.legacyReads)).toEqual([]);
});

test('bounded history quarantines a legacy reasoning-only reply before and after loading details',async({page})=>{
  await page.addInitScript(()=>localStorage.setItem('timeline-reasoning-only','1'));
  await page.goto('/chat/history');
  const privateReasoning = page.getByText('Private intermediate reasoning from a legacy interrupted turn.',{exact:true});
  await expect(page.getByText('The model ended before producing a final answer. Its reasoning was kept separate.')).toBeVisible();
  await expect(page.getByRole('button',{name:'Generate final answer'})).toBeVisible();
  await expect(privateReasoning).toHaveCount(0);
  expect(await page.evaluate(()=>(window as any).__TIMELINE_TEST__.detailRequests)).toEqual([]);
  await page.getByTestId('deferred-activity-u10000').getByRole('button').click();
  await expect(page.getByRole('button',{name:/Thinking completed/})).toBeVisible();
  await expect(privateReasoning).toBeVisible();
  await expect(privateReasoning).toHaveCount(1);
  await expect(page.getByRole('button',{name:'Generate final answer'})).toBeVisible();
  await expect(page.getByLabel('Assistant response').filter({hasText:'Private intermediate reasoning'})).toHaveCount(0);
});
