import { useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { ToolCallCard } from '../../src/components/chat/ToolCallCard';
import { streamStore } from '../../src/lib/streamStore';
import { useStreamTool } from '../../src/lib/useStreamSelector';
import type { AgentRunEvent } from '../../src/types/conversation';

const conversationId = 'entity-probe';
let sequence = 0;
function emit(index:number,note:string) {
  const runEvent: AgentRunEvent = { version:2,runId:'entity-run',turnId:'entity-turn',eventSeq:++sequence,kind:note?'toolProgress':'toolStarted',phase:'tooling',visibility:'user',persistence:'durable',displayKind:'tool',importance:'normal',label:'fixture_tool',createdAt:new Date().toISOString(),payload:{run:{callId:`call${index}`,toolName:`fixture_tool_${index}`,status:'running',arguments:'{}',progressNote:note,content:'Retained content',renderKind:'generic',owner:{id:'fixture',name:'Fixture',capability:'test',description:''},capabilities:{inputStreaming:'none',renderKind:'generic',readOnly:true,destructive:false,concurrencySafe:true,interruptBehavior:'cancel',resourceKeys:[]}}} };
  streamStore.dispatch(conversationId,{conversationId,runEvent});
}
function Probe({index}:{index:number}) {
  const tool=useStreamTool(conversationId,`call${index}`);
  const renders=useRef(0);renders.current++;
  const [open,setOpen]=useState(false);
  return <section data-testid={`probe-${index}`} data-renders={renders.current} className="space-y-2 rounded border p-3">
    <button onClick={()=>setOpen(value=>!value)}>Toggle retained panel {index}</button>
    {open&&<div data-testid={`retained-${index}`}>Retained local panel</div>}
    {tool&&<ToolCallCard {...tool} trace />}
    <output>{tool?.progressNote}</output>
  </section>;
}
export function renderToolEntities() {
  localStorage.setItem('nexa-locale','en');
  streamStore.startStream(conversationId);
  for(let index=0;index<512;index++)emit(index,'');
  Object.assign(window,{__ENTITY_TEST__:{ update(){for(let index=0;index<1000;index++)emit(511,`Update ${index}`);}}});
  const root=document.createElement('div');document.body.append(root);
  createRoot(root).render(<I18nProvider><main className="space-y-4 p-8"><input aria-label="Independent draft"/><Probe index={0}/><Probe index={511}/></main></I18nProvider>);
}
