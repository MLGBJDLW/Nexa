import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { RemoteChat } from '../../src/features/remote/RemoteChat';
import type { RemoteChatClient, RemoteEnvelope } from '../../src/features/remote/remoteClient';
import '../../../mobile/src/mobile.css';
export function renderMobileContinuity() {
  localStorage.setItem('nexa-locale', 'en');
  localStorage.removeItem('nexa.remote.chat.mobile-fixture');
  const actions: {method:string;params:any}[] = [];
  const listeners = new Set<(event:RemoteEnvelope)=>void>();
  let chats = [{id:'old',title:'Previous work',model:'model'}];
  let archived: typeof chats = [];
  let holdArchive = false;
  let releaseArchive: (()=>void) | null = null;
  let run: any = null;
  const events: any[] = [];
  const emit = (kind: string, payload: unknown) => {
    const event = {version:2,runId:'run',turnId:'turn',eventSeq:events.length+1,kind,visibility:'user',label:'Working',payload};
    events.push(event); listeners.forEach(listener=>listener({event:'agent://run-event',payload:{conversationId:'created',runEvent:event}}));
  };
  const client = {
    paired:{token:'',device:{id:'fixture',name:'Phone'},manifest:{serverId:'mobile-fixture',endpoints:[],reconnectGraceSeconds:0}},
    state:{phase:'connected',endpoint:null}, conversationManagementAvailable:true, voiceAvailable:false, modelPickerAvailable:false, composerOptionsAvailable:false,
    subscribe:(listener:(event:RemoteEnvelope)=>void)=>{listeners.add(listener);return()=>listeners.delete(listener);},
    subscribeConnection:()=>()=>{},
    async rpc(method:string,params:any={}) {
      actions.push({method,params});
      if (method==='connections.list') return [{id:'connection',name:'Provider',model:'model',isDefault:true}];
      if (method==='chat.list') return {items:chats,nextCursor:null};
      if (method==='chat.archived') { const page={items:[...archived],nextCursor:null}; if(holdArchive) {holdArchive=false;return new Promise(resolve=>{releaseArchive=()=>{releaseArchive=null;resolve(page);};});} return page; }
      if (method==='chat.restore') {chats.push(...archived.filter(item=>item.id===params.conversationId));archived=archived.filter(item=>item.id!==params.conversationId);return null;}
      if (method==='chat.create') { const item={id:'created',title:'New work',model:'model'};chats=[item,...chats];return item; }
      if (method==='chat.start') { run={id:'run',status:'running'};return {}; }
      if (method==='chat.stop') { run.status='cancelled';emit('done',{status:'cancelled'});return {}; }
      if (method==='chat.resume') return {run:params.conversationId==='created'?run:null,events:events.filter(event=>event.eventSeq>(params.afterSequence||0)),hasMore:false,nextSequence:events.length,durableHighWater:events.length};
      if (method==='chat.read') return {messages:[],beforeOrder:null};
      if (method==='chat.archive'||method==='chat.delete') {if(method==='chat.archive') archived.push(...chats.filter(item=>item.id===params.conversationId)); chats=chats.filter(item=>item.id!==params.conversationId);if(method==='chat.delete') archived=archived.filter(item=>item.id!==params.conversationId);return 1;}
      if (method==='approvals.list'||method==='interactions.list') return [];
      throw new Error(method);
    },
  } as unknown as RemoteChatClient;
  (window as any).__MOBILE_CONTINUITY__={actions,emit,holdArchive:()=>{holdArchive=true;},archivePending:()=>!!releaseArchive,releaseArchive:()=>releaseArchive?.()};
  const host=document.createElement('div');host.className='mobile-app';host.style.cssText='position:fixed;inset:0;z-index:1000;background:var(--color-surface-0)';document.body.appendChild(host);
  createRoot(host).render(<I18nProvider><RemoteChat client={client} initialDraft="" onDraftConsumed={()=>{}} mobile={{heading:'Phone',drawerFooter:null,caption:'',onConfigure:()=>{},onFiles:()=>{}}}/></I18nProvider>);
}
