import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { TaskBoard } from '../../src/components/chat/TaskBoard';
import type { ToolCallEvent } from '../../src/lib/streaming/protocol';
import type { ConversationMessage } from '../../src/types/conversation';

const tasks = ['Audit renderer', 'Verify cancellation'];
const spawn = tasks.map((title, index): ToolCallEvent => ({
  callId: `spawn-${index}`, toolName: 'spawn_subagent', status: 'done', arguments: '{}',
  argsStatus: 'done', argsBytes: 2, artifacts: { kind: 'subagent_result', id: `agent-${index}`,
    task: `${title}\n${'PRIVATE full task instructions '.repeat(100)}`, status: 'running', result: '', toolEvents: [] },
}));
const closed = tasks.map((task, index): ToolCallEvent => ({
  callId: `close-${index}`, toolName: 'close_subagent', status: 'done', arguments: '{}',
  argsStatus: 'done', argsBytes: 2, artifacts: { kind: 'subagent_closed', worker: {
    agentId: `agent-${index}`, parentCallId: `spawn-${index}`, task,
    status: index === 0 ? 'completed' : 'cancelled', result: null,
  } },
}));

function Fixture() {
  const [stage, setStage] = useState(0);
  const calls = stage === 0 ? spawn : [...spawn, ...closed];
  const messages: ConversationMessage[] = stage < 2 ? [] : calls.flatMap((call, index) => {
    const base = { conversationId: 'capsule', content: '', toolCallId: null, artifacts: null,
      tokenCount: 0, createdAt: '2026-09-16T00:00:00Z', sortOrder: index * 2, thinking: null, imageAttachments: null };
    return [
      { ...base, id: `assistant-${index}`, role: 'assistant' as const, toolCalls: [{ id: call.callId, name: call.toolName, arguments: '{}' }] },
      { ...base, id: `tool-${index}`, role: 'tool' as const, toolCallId: call.callId, toolCalls: [], artifacts: call.artifacts ?? null, sortOrder: index * 2 + 1 },
    ];
  });
  return <div className="relative h-screen bg-surface-0 text-text-primary">
    <div className="absolute bottom-8 left-8 flex gap-4">
      <button onClick={() => setStage(1)}>Close all workers</button>
      <button onClick={() => setStage(2)}>Reopen saved turn</button>
    </div>
    <TaskBoard messages={messages} toolCalls={stage < 2 ? calls : []} />
  </div>;
}

export function renderSubagentCapsule() {
  const host = document.createElement('div');
  host.style.cssText = 'position:fixed;inset:0;z-index:1000';
  document.body.appendChild(host);
  createRoot(host).render(<I18nProvider><Fixture /></I18nProvider>);
}
