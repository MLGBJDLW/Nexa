import { expect, test } from '@playwright/test';

test.use({ launchOptions: { args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] } });
test.beforeEach(async ({ page, context }) => {
  await context.grantPermissions(['microphone', 'camera']);
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale', 'en');
    localStorage.setItem('last-health-check-at', String(Date.now()));
    localStorage.setItem('last-insights-at', String(Date.now()));
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, {event: string; handler: number}>();
    let seq=1; let lid=1; let delayed=false; let holdReady=false; let release:(()=>void)|undefined;
    const state={ audio:0, frames:0, stopped:0, starts:0, snapshots:0, earlyAudio:0, tracks:[] as MediaStreamTrack[], snapshot:{id:'live-1',mode:'qwenRealtime',model:'qwen3.5-omni-flash-realtime',phase:'connecting',sampleRate:16000,startedAt:new Date().toISOString(),sequence:0,entries:[] as {id:string;role:string;text:string;atMs:number;complete:boolean}[],metrics:{framesReceived:0,framesSubmitted:0,framesReplaced:0,audioMs:0,lastResponseMs:null as number|null,omittedEntries:0},error:null as string|null} };
    const getUserMedia=navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
    navigator.mediaDevices.getUserMedia=async constraints=>{const stream=await getUserMedia(constraints);state.tracks.push(...stream.getTracks());return stream;};
    const emit=(event:Record<string,unknown>)=>{
      const payload={...event,sessionId:state.snapshot.id,sequence:++state.snapshot.sequence};
      for(const [id,listener] of listeners) if(listener.event==='live:event')callbacks.get(listener.handler)?.({event:'live:event',id,payload});
    };
    const invoke=async (cmd:string,args:Record<string,unknown>={})=>{
      switch(cmd){
        case 'plugin:event|listen':{const id=lid++;listeners.set(id,{event:String(args.event),handler:Number(args.handler)});return id;}
        case 'plugin:event|unlisten':listeners.delete(Number(args.eventId));return null;
        case 'live_connections_cmd':return [{id:'qwen',name:'Qwen',model:'qwen-vl',isDefault:true,nativeProtocols:['qwenRealtime'],vision:'supported'},{id:'summary',name:'Summary',model:'deepseek',isDefault:false,nativeProtocols:[],vision:'unsupported'}];
        case 'start_live_cmd':state.starts++;if(delayed)await new Promise<void>(resolve=>{release=resolve;});return structuredClone(state.snapshot);
        case 'live_snapshot_cmd':state.snapshots++;if(!holdReady)state.snapshot.phase='listening';return structuredClone(state.snapshot);
        case 'live_audio_cmd':state.audio++;if(state.snapshot.phase==='connecting')state.earlyAudio++;return null;
        case 'live_frame_cmd':state.frames++;state.snapshot.metrics.framesSubmitted=state.frames;state.snapshot.metrics.lastResponseMs=85;state.snapshot.entries=[{id:'observation',role:'assistant',text:'A diagram is visible. The speaker proposes a Friday deadline.',atMs:1400,complete:false}];emit({type:'entry',entry:state.snapshot.entries[0]});emit({type:'metrics',metrics:state.snapshot.metrics});return null;
        case 'stop_live_cmd':state.stopped++;state.snapshot.phase='stopped';return structuredClone(state.snapshot);
        case 'summarize_live_cmd':return {snapshot:structuredClone(state.snapshot),summary:'Decision: review the diagram by Friday. Owner remains unconfirmed.'};
        case 'list_live_records_cmd':return state.stopped?[{id:state.snapshot.id,startedAt:state.snapshot.startedAt,model:state.snapshot.model}]:[];
        case 'load_live_record_cmd':return {snapshot:structuredClone(state.snapshot),summary:null};
        case 'list_agent_configs_cmd':case 'list_sources':case 'list_conversations_cmd':case 'list_personas_cmd':case 'list_projects_cmd':case 'list_skills_cmd':return [];
        default:return null;
      }
    };
    Object.assign(window,{__live:state,__delayLive:()=>{delayed=true;},__releaseLive:()=>release?.(),__disconnectLive:()=>emit({type:'state',phase:'error',error:'Fixture provider disconnected'})});
    Object.assign(window,{
      __holdReadyLive:()=>{holdReady=true;},
      __failLiveSetup:()=>{state.snapshot.phase='error';state.snapshot.error='Setup failed';emit({type:'state',phase:'error',error:'Setup failed'});},
      __resumeReadyLive:()=>{holdReady=false;state.snapshot.phase='connecting';state.snapshot.error=null;},
    });
    Object.assign(window,{__TAURI_INTERNALS__:{invoke,metadata:{currentWindow:{label:'main'}},transformCallback:(cb:(event:unknown)=>void)=>{const id=seq++;callbacks.set(id,cb);return id;},unregisterCallback:(id:number)=>callbacks.delete(id),convertFileSrc:(path:string)=>path},__TAURI_EVENT_PLUGIN_INTERNALS__:{unregisterListener:(_:string,id:number)=>listeners.delete(id)}});
  });
});

async function chooseQwen(page: import('@playwright/test').Page) {
  await page.goto('/live');
  await page.getByLabel('Analysis mode').selectOption('qwenRealtime');
  await page.getByLabel('Qwen workspace endpoint', {exact:true}).fill('wss://fixture.cn-beijing.maas.aliyuncs.com/api-ws/v1/realtime');
  await page.getByLabel('Visual input').selectOption('camera');
  await page.getByLabel('Observation interval (seconds)',{exact:true}).fill('1');
}
const state = (page:import('@playwright/test').Page)=>page.evaluate(()=>{const s=(window as unknown as {__live:{audio:number;frames:number;stopped:number;starts:number;snapshots:number;earlyAudio:number;tracks:MediaStreamTrack[]}}).__live;return {...s,tracks:s.tracks.map(track=>track.readyState)};});

test('can restart after a terminal event during the initial ready wait',async({page})=>{
  await chooseQwen(page);
  await page.evaluate(()=>(window as unknown as {__holdReadyLive:()=>void}).__holdReadyLive());
  await page.getByRole('button',{name:'Start Live',exact:true}).click();
  await expect.poll(async()=>(await state(page)).snapshots).toBeGreaterThan(0);
  await page.evaluate(()=>(window as unknown as {__failLiveSetup:()=>void}).__failLiveSetup());
  await expect(page.getByRole('alert')).toContainText('Setup failed');
  await expect.poll(async()=>(await state(page)).tracks.every(track=>track==='ended')).toBe(true);
  expect((await state(page)).audio).toBe(0);
  await page.evaluate(()=>(window as unknown as {__resumeReadyLive:()=>void}).__resumeReadyLive());
  await page.getByRole('button',{name:'Start Live',exact:true}).click();
  await expect.poll(async()=>(await state(page)).starts).toBe(2);
  await expect.poll(async()=>(await state(page)).audio).toBeGreaterThan(1);
  await page.getByRole('button',{name:'Stop capture'}).click();
});

test('captures real browser PCM and frames only after ready, then stops and summarizes',async({page})=>{
  await chooseQwen(page);
  expect((await state(page)).tracks).toEqual([]);
  await page.getByRole('button',{name:'Start Live',exact:true}).click();
  await expect.poll(async()=>(await state(page)).audio).toBeGreaterThan(2);
  await expect(page.getByText('A diagram is visible. The speaker proposes a Friday deadline.')).toBeVisible();
  expect((await state(page)).earlyAudio).toBe(0);
  await page.screenshot({path:'.artifacts/live-observation-desktop.png',fullPage:true});
  await page.getByRole('button',{name:'Stop capture'}).click();
  await expect.poll(async()=>(await state(page)).tracks.every(track=>track==='ended')).toBe(true);
  const stoppedAudio=(await state(page)).audio;
  await page.waitForTimeout(350);
  expect((await state(page)).audio).toBe(stoppedAudio);
  await page.getByLabel('Summary connection').selectOption('summary');
  await page.getByRole('button',{name:'Create summary'}).click();
  await expect(page.getByTestId('live-summary')).toContainText('Owner remains unconfirmed');
});

test('cancels a pending start and closes media when the late session arrives',async({page})=>{
  await chooseQwen(page);
  await page.evaluate(()=>(window as unknown as {__delayLive:()=>void}).__delayLive());
  await page.getByRole('button',{name:'Start Live',exact:true}).click();
  await expect.poll(async()=>(await state(page)).tracks.length).toBeGreaterThan(0);
  await page.getByRole('button',{name:'Stop capture'}).click();
  await page.evaluate(()=>(window as unknown as {__releaseLive:()=>void}).__releaseLive());
  await expect.poll(async()=>(await state(page)).stopped).toBe(1);
  expect((await state(page)).tracks.every(track=>track==='ended')).toBe(true);
  expect((await state(page)).audio).toBe(0);
});

test('releases microphone and camera on provider failure and on navigation',async({page})=>{
  await chooseQwen(page);
  await page.getByRole('button',{name:'Start Live',exact:true}).click();
  await expect.poll(async()=>(await state(page)).audio).toBeGreaterThan(1);
  await page.evaluate(()=>(window as unknown as {__disconnectLive:()=>void}).__disconnectLive());
  await expect(page.getByRole('alert')).toContainText('Fixture provider disconnected');
  await expect.poll(async()=>(await state(page)).tracks.every(track=>track==='ended')).toBe(true);
  await page.getByRole('button',{name:'Start Live',exact:true}).click();
  await expect.poll(async()=>(await state(page)).tracks.some(track=>track==='live')).toBe(true);
  await page.getByRole('link',{name:'Knowledge',exact:true}).click();
  await expect.poll(async()=>(await state(page)).tracks.every(track=>track==='ended')).toBe(true);
});

test('keeps Live controls readable at phone width',async({page})=>{
  await page.setViewportSize({width:430,height:932});
  await chooseQwen(page);
  await expect(page.getByRole('button',{name:'Start Live',exact:true})).toBeVisible();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBe(true);
  await page.screenshot({path:'.artifacts/live-observation-phone.png',fullPage:true});
});
