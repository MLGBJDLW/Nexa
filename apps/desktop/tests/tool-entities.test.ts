import { createDefaultState, capStreamCollections } from '../src/lib/streaming/state';
import { applyToolRunEvent, createToolCall } from '../src/lib/streaming/toolProjection';
import { applyStreamResetProjection, applyTerminalProjection } from '../src/lib/streaming/terminalProjection';
import { extractMcpResult, mcpExternalLink, mcpInlineMedia } from '../src/lib/mcpResult';
import type { ToolRunItem } from '../src/types/conversation';

function assert(condition: unknown, message: string): asserts condition { if (!condition) throw new Error(message); }
const state = createDefaultState();
const run: ToolRunItem = { callId: '',toolName: 'run_shell',status: 'running',renderKind: 'commandExecution',owner: { id:'test',name:'Test',capability:'test',description:'' },capabilities: { inputStreaming:'none',renderKind:'commandExecution',readOnly:true,destructive:false,concurrencySafe:true,interruptBehavior:'cancel',resourceKeys:[] } };
state.toolCalls = Array.from({ length: 512 },(_,index) => createToolCall({ callId: `call${index}`, toolName: 'run_shell', status: 'running' }));
state.streamRounds = Array.from({ length: 128 },(_,index) => ({ id: `round${index}`, reply: '', toolCalls: state.toolCalls.slice(index*4,index*4+4) }));
state.traceEvents = state.toolCalls.map((toolCall,index) => ({ id: `trace${index}`, kind: 'tool' as const, toolCall }));
const tools = state.toolCalls;
const rounds = state.streamRounds;
const trace = state.traceEvents;
const unrelated = state._tools.get('call0');
let unaffectedChanges = 0;
const start = performance.now();
for (let index = 0; index < 10_000; index++) {
  applyToolRunEvent(state,{ ...run,callId: 'call511',progressNote: `progress ${index}` });
  if (state._tools.get('call0') !== unrelated) unaffectedChanges++;
}
console.log(`tool entity projection: 10000 updates at 512 tools in ${(performance.now()-start).toFixed(2)} ms; unrelated selector changes: ${unaffectedChanges}`);
assert(unaffectedChanges === 0,'unrelated tool selector must not change on progress');
assert(state.toolCalls.filter((tool,index) => tool !== tools[index]).length === 1,'progress replaces only one entity');
assert(state.streamRounds.filter((round,index) => round !== rounds[index]).length === 1,'only the owning round projection changes');
assert(state.traceEvents.filter((event,index) => event !== trace[index]).length === 1,'only the matching trace projection changes');
const tool = state._tools.get('call511');
const event = state.traceEvents[511];
assert(state.streamRounds[127].toolCalls[3] === tool && event.kind === 'tool' && event.toolCall === tool,'all views resolve the same entity');
applyStreamResetProjection(state,'retry',{ clearTools: true });
assert(state.toolCalls.length === 0 && state.streamRounds[0].toolCalls[0].status === 'cancelled','retry clears active IDs and preserves terminal history');
applyToolRunEvent(state,{ ...run,callId: 'new-call' });
applyTerminalProjection(state,{ toolStatus: 'timedOut',message: 'timeout',traceTone: 'error' });
assert(state._tools.get('new-call')?.status === 'timedOut','terminal state settles the new entity');
capStreamCollections(state);
assert(state._tools.byId.size <= 513,'pruning releases unreferenced entities');

const reused = createDefaultState();
applyToolRunEvent(reused,{ ...run,callId:'provider-reused',arguments:'old-file' });
applyStreamResetProjection(reused,'new provider sample',{clearTools:true});
const retiredRound = reused.streamRounds[0];
const retiredTrace = reused.traceEvents.find(event=>event.kind==='tool');
applyToolRunEvent(reused,{ ...run,callId:'provider-reused',arguments:'new-file' });
applyToolRunEvent(reused,{ ...run,callId:'provider-reused',arguments:'new-file',status:'completed',content:'new result' });
assert(reused.streamRounds[0] === retiredRound && retiredRound.toolCalls[0].status === 'cancelled' && retiredRound.toolCalls[0].arguments === 'old-file','reusing a provider call ID cannot rewrite a retired round');
assert(reused.traceEvents[0] === retiredTrace && reused.traceEvents.filter(event=>event.kind==='tool').length === 2,'each provider sample retains its own tool trace occurrence');
assert(reused.toolCalls[0].arguments === 'new-file' && reused.toolCalls[0].status === 'done','active call ID resolves the new occurrence');
assert(reused._tools.byId.get(reused._tools.key(retiredRound.toolCalls[0])!) === retiredRound.toolCalls[0],'an occurrence subscription resolves the retired card even while the same call ID is active again');

assert(mcpExternalLink('javascript:alert(1)') === null && mcpExternalLink('file:///private') === null,'MCP links must not execute code or open local resources');
assert(mcpExternalLink('https://example.org/result') === 'https://example.org/result','explicit remote resource link remains usable');
assert(mcpInlineMedia('YWJj','image/svg+xml','image') === null,'active image formats are not accepted');
assert(extractMcpResult({ kind:'mcpToolResult',version:1,contentBlocks:[{ type:'text',text:'text' },{ type:'resource_link',uri:'https://example.org',name:'source' }],structuredContent:{ answer:42 },notices:[] })?.contentBlocks.length === 2,'typed MCP artifact survives history projection');
console.log('normalized tool entity and MCP presentation contracts passed');
