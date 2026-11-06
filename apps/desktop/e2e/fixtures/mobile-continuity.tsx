import { createRoot } from 'react-dom/client';
import { applyTheme, getInitialTheme } from '../../src/lib/theme';
import { I18nProvider } from '../../src/i18n';
import { RemoteChat } from '../../src/features/remote/RemoteChat';
import type { RemoteChatClient, RemoteEnvelope } from '../../src/features/remote/remoteClient';
import { phoneChatProjects } from '../../../mobile/src/phoneChatProjects';
import '../../../mobile/src/mobile.css';
export function renderMobileContinuity() {
  localStorage.setItem('nexa-locale', 'en');
  localStorage.removeItem('nexa.remote.chat.mobile-fixture');
  const actions: {method:string;params:any}[] = [];
  const listeners = new Set<(event:RemoteEnvelope)=>void>();
  let chats = [{id:'old',title:'Previous work',model:'model'}];
  const params = new URLSearchParams(location.search);
  const projectsFixture = params.has('projects');
  const hostId = params.get('host');
  if (projectsFixture) {
    let interruptNewChatMove = params.has('interrupt');
    applyTheme(getInitialTheme());
    document.documentElement.dataset.remote = 'true';
    if (params.has('paged')) {
      chats.push({id:'middle',title:'Middle chat',model:'model'}, {id:'earliest',title:'Earliest chat',model:'model'});
    }
    // Simulate only the native store: the drawer and the Tauri command adapter
    // are production code. Persist through page reload and isolate host IDs.
    (window as any).__TAURI_INTERNALS__ = { invoke: async (command: string, args: {
      hostId: string | null; action: string; projectId?: string | null; name?: string; conversationId?: string;
    }) => {
      const key = 'fixture.chatProjects.' + (args.hostId ?? 'phone');
      const state: {projects: {id:string;name:string}[];memberships: Record<string,string>} =
        JSON.parse(localStorage.getItem(key) ?? '{"projects":[],"memberships":{}}');
      if (command === 'mobile_project_chats') return chats.filter(chat => state.memberships[chat.id] === args.projectId);
      if (command !== 'mobile_chat_projects') throw new Error(command);
      if (args.action === 'create') state.projects.push({id:args.projectId!,name:args.name!});
      if (args.action === 'move' && args.conversationId) {
        if (interruptNewChatMove && args.conversationId === 'created') {
          interruptNewChatMove = false;
          throw new Error('Project move interrupted');
        }
        if (args.projectId) state.memberships[args.conversationId] = args.projectId;
        else delete state.memberships[args.conversationId];
      }
      if (args.action === 'rename') state.projects.find(item => item.id === args.projectId)!.name = args.name!;
      if (args.action === 'delete') {
        state.projects = state.projects.filter(item => item.id !== args.projectId);
        state.memberships = Object.fromEntries(Object.entries(state.memberships).filter(([, id]) => id !== args.projectId));
      }
      localStorage.setItem(key, JSON.stringify(state));
      return state;
    }};
  }
  let archived: typeof chats = [];
  let holdArchive = false;
  let releaseArchive: (()=>void) | null = null;
  let run: any = null;
  let messages: any[] = [];
  const events: any[] = [];
  const emit = (kind: string, payload: unknown, presentation: {displayKind?:string;label?:string} = {}) => {
    const event = {version:2,runId:'run',turnId:'turn',eventSeq:events.length+1,kind,visibility:'user',label:activityFixture?'处理中':'Working',payload,...presentation};
    events.push(event); listeners.forEach(listener=>listener({event:'agent://run-event',payload:{conversationId:'created',runEvent:event}}));
  };
  const activityFixture = new URLSearchParams(location.search).has('activity');
  let pendingApproval = false;
  if (activityFixture) {
    applyTheme(getInitialTheme());
    document.documentElement.dataset.remote = 'true';
    localStorage.setItem('nexa-locale', 'zh-CN');
    chats = [{id:'created',title:'Report review',model:'model'}];
    localStorage.setItem('nexa.remote.chat.mobile-fixture', 'created');
    messages = [
      { id:'user1', role:'user', content:'核对第一份报表。', sortOrder:1 },
      { id:'old-step', role:'assistant', content:'', thinking:'读取第一份报表。', intermediate:true, toolCalls:[{id:'old-file',name:'read_file'}], sortOrder:2 },
      { id:'steering', role:'user', content:'请重点核对合计金额。', continuesTurn:true, sortOrder:3 },
      { id:'old-check', role:'assistant', content:'', thinking:'核对表格里的金额。', intermediate:true, toolCalls:[{id:'old-search',name:'search_table'}], sortOrder:4 },
      { id:'old-final', role:'assistant', content:'第一份报表的合计金额一致。', sortOrder:5 },
      { id:'user2', role:'user', content:'第二份报表有几个部分？', sortOrder:6 },
    ].map(row => ({...row,totalChars:row.content.length}));
    const persisted = sessionStorage.getItem('fixture-completed-turn');
    if (persisted) messages.push(...JSON.parse(persisted));
    else {
      run = {id:'run',status:'running',userMessageId:'user2'};
      pendingApproval = true;
      emit('outputSnapshot',{channel:'thinking',blockId:'think',text:'读取第二份报表的目录。'});
      emit('toolStarted',{run:{callId:'current-file',toolName:'read_section',status:'running'}});
      emit('toolCompleted',{run:{callId:'current-file',toolName:'read_section',status:'completed',content:'已找到三个部分。'}});
      emit('outputSnapshot',{channel:'answer',blockId:'answer',text:'第二份报表有三个部分。'});
    }
  }
  const pagedActivity = params.get('pagedActivity');
  if (pagedActivity) {
    chats = [{id:'created',title:'Long task',model:'model'}];
    localStorage.setItem('nexa.remote.chat.mobile-fixture', 'created');
    messages = Array.from({length:30}, (_, index) => ({
      id:'record-' + index, role:'assistant', content:'', totalChars:0, intermediate:true, sortOrder:10 + index,
      toolCalls:[{id:'call-' + index, name:'saved_step'}], ...(index === 0 ? {precedingUserId:'original'} : {}),
    }));
    run = {id:'run',status:'running',userMessageId:pagedActivity === 'same' ? 'original' : 'newer'};
    emit('toolStarted', {run:{callId:'live-file',toolName:'active_step',status:'running'}});
  }
  const composerFixture = new URLSearchParams(location.search).has('composer');
  let speechRequest: { id?: string; resolve: (value: string) => void; reject: (error: Error) => void } | undefined;
  const nativeSpeech = {
    listen: (_language: string, id?: string) => new Promise<string>((resolve, reject) => { actions.push({method:'speech.listen',params:{id}}); speechRequest = { id, resolve, reject }; }),
    finish: async (id?: string) => { actions.push({method:'speech.finish',params:{id}}); if (speechRequest && speechRequest.id === id) { speechRequest.resolve('Hello from dictation'); speechRequest = undefined; } },
    cancel: async (id?: string) => { actions.push({method:'speech.cancel',params:{id}}); if (speechRequest && speechRequest.id === id) { speechRequest.reject(new Error('speech_cancelled')); speechRequest = undefined; } },
    speak: async () => {}, stop: async () => {},
  };
  const client = {
    paired:{token:'',device:{id:'fixture',name:'Phone'},manifest:{serverId:'mobile-fixture',endpoints:[],reconnectGraceSeconds:0,capabilities:['chat-steering-v1']}},
    state:{phase:'connected',endpoint:null}, conversationManagementAvailable:true, archivedDeletionAvailable:true, voiceAvailable:false, modelPickerAvailable:false, composerOptionsAvailable:composerFixture, approvalModeAvailable:composerFixture, nativeSpeech: composerFixture ? nativeSpeech : undefined,
    subscribe:(listener:(event:RemoteEnvelope)=>void)=>{listeners.add(listener);return()=>listeners.delete(listener);},
    subscribeConnection:()=>()=>{},
    async rpc(method:string,params:any={}) {
      actions.push({method,params});
      if (method==='connections.list') return [{id:'connection',name:'Provider',model:'model',isDefault:true}];
      if (method==='chat.list') {
        if (!projectsFixture || !new URLSearchParams(location.search).has('paged')) return {items:chats,nextCursor:null};
        const start = params.before ? chats.findIndex(item => item.id === params.before) + 1 : 0;
        return {items:chats.slice(start,start+1),nextCursor: start+1 < chats.length ? chats[start].id : null};
      }
      if (method==='chat.archived') { const start=params.before?archived.findIndex(item=>item.id===params.before)+1:0; const items=archived.slice(start,start+2); const page={items,nextCursor:start+2<archived.length?items.at(-1)!.id:null}; if(holdArchive) {holdArchive=false;return new Promise(resolve=>{releaseArchive=()=>{releaseArchive=null;resolve(page);};});} return page; }
      if (method==='chat.restore') {chats.push(...archived.filter(item=>item.id===params.conversationId));archived=archived.filter(item=>item.id!==params.conversationId);return null;}
      if (method==='chat.create') { const item={id:'created',title:'New work',model:'model'};chats=[item,...chats];return item; }
      if (method==='chat.start') { run={id:'run',status:'running'};return {}; }
      if (method==='chat.steer') {
        if (params.runId !== run?.id || params.conversationId !== 'created') throw new Error('Wrong steering target');
        messages.push({id:'clarification',role:'user',steering:true,continuesTurn:true,content:params.message,totalChars:[...params.message].length,sortOrder:7});
        emit('status', {}, {displayKind:'steering', label:params.message});
        return {};
      }
      if (method==='chat.stop') { run.status='cancelled';emit('done',{status:'cancelled'});return {}; }
      if (method==='chat.resume') return {run:params.conversationId==='created'?run:null,events:events.filter(event=>event.eventSeq>(params.afterSequence||0)),hasMore:false,nextSequence:events.length,durableHighWater:events.length};
      if (method==='chat.read') return {messages,beforeOrder:pagedActivity ? 10 : null};
      if (method==='chat.deleteArchived') {const previous=archived.length;archived=archived.filter(item=>item.id!==params.conversationId);return previous-archived.length;}
      if (method==='chat.archive'||method==='chat.delete') {if(method==='chat.archive') archived.push(...chats.filter(item=>item.id===params.conversationId)); chats=chats.filter(item=>item.id!==params.conversationId);if(method==='chat.delete') archived=archived.filter(item=>item.id!==params.conversationId);return 1;}
      if (method==='approvals.list') return pendingApproval ? [{runId:'run',request:{id:'check',toolName:'verify_report',reason:'允许核对第二份报表吗？',targetValue:'report',argumentsPreview:'{}'}}] : [];
      if (method==='approvals.respond' && activityFixture) {
        pendingApproval = false;
        const current = [
          {id:'step2',role:'assistant',content:'',totalChars:0,sortOrder:7,intermediate:true,thinking:'读取第二份报表的目录。',toolCalls:[{id:'current-file',name:'read_section'}]},
          {id:'final2',role:'assistant',content:'第二份报表有三个部分。',totalChars:'第二份报表有三个部分。'.length,sortOrder:8},
        ];
        messages.push(...current);
        sessionStorage.setItem('fixture-completed-turn', JSON.stringify(current));
        run = {id:'run',status:'completed',userMessageId:'user2',finalMessageId:'final2'};
        emit('done',{assistantMessageId:'final2',status:'completed'});
        return {};
      }
      if (method==='interactions.list') return [];
      throw new Error(method);
    },
  } as unknown as RemoteChatClient;
  (window as any).__MOBILE_CONTINUITY__={actions,emit,removeArchived:(id:string)=>{archived=archived.filter(item=>item.id!==id);},restoreArchived:(id:string)=>{chats.push(...archived.filter(item=>item.id===id));archived=archived.filter(item=>item.id!==id);},seedArchives:()=>{archived=['a','b','c','d','e'].map(id=>({id,title:'Archived '+id,model:'model'}));},finish:(content='Final report: all three sections are consistent.')=>{messages=[{id:'intro',role:'assistant',content:'I will inspect the report.',totalChars:26,sortOrder:1,intermediate:true},{id:'final',role:'assistant',content,totalChars:[...content].length,sortOrder:2}];run={...run,status:'completed',finalMessageId:'final'};emit('done',{status:'completed'});},holdArchive:()=>{holdArchive=true;},archivePending:()=>!!releaseArchive,releaseArchive:()=>releaseArchive?.()};
  const host=document.createElement('div');host.className='mobile-app';host.style.cssText='position:fixed;inset:0;z-index:1000;background:var(--color-surface-0)';document.body.appendChild(host);
  createRoot(host).render(<I18nProvider><RemoteChat client={client} initialDraft="" onDraftConsumed={()=>{}} mobile={{heading:activityFixture ? 'Nexa' : 'Phone',drawerFooter:null,caption:'',onConfigure:()=>{},onFiles:()=>{},
    projectStore: projectsFixture ? phoneChatProjects(hostId) : undefined}}/></I18nProvider>);
}
