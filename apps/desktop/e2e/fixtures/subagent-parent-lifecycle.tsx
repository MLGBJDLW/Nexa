import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { ChatMessages } from '../../src/features/chat/ChatMessages';
import { TaskBoard } from '../../src/components/chat/TaskBoard';
import type { ActivityEvent, ConversationMessage } from '../../src/types/conversation';
import type { ToolCallEvent } from '../../src/lib/streaming/protocol';

function Fixture() {
  const [phase, setPhase] = useState<'queued' | 'running' | 'cancelling' | 'completed'>('queued');
  const [mode, setMode] = useState<'live' | 'settled' | 'historical'>('live');
  const parentActive = mode === 'live';
  const activityEvents: ActivityEvent[] = [{
    activityId: 'accepted-worker', seq: 1, timestamp: '2026-09-29T00:00:00Z', kind: 'progress',
    payload: { subagentEvent: 'spawned', agentId: 'accepted-worker', detail: { task: 'Inspect delegated evidence' } },
  }, {
    activityId: 'accepted-worker', seq: 2, timestamp: '2026-09-29T00:00:01Z', kind: 'progress',
    payload: { subagentEvent: 'queued', agentId: 'accepted-worker', detail: { status: 'queued' } },
  }];
  if (phase !== 'queued') activityEvents.push({
    activityId: 'accepted-worker', seq: 3, timestamp: '2026-09-29T00:00:02Z', kind: 'progress',
    payload: { subagentEvent: 'progress', agentId: 'accepted-worker', detail: { status: 'running' } },
  });
  if (phase === 'cancelling') activityEvents.push({
    activityId: 'accepted-worker', seq: 4, timestamp: '2026-09-29T00:00:03Z', kind: 'progress',
    payload: { subagentEvent: 'progress', agentId: 'accepted-worker', detail: { status: 'cancelling' } },
  });
  if (phase === 'completed') activityEvents.push({
    activityId: 'accepted-worker', seq: 5, timestamp: '2026-09-29T00:00:04Z', kind: 'completed',
    payload: { subagentEvent: 'completed', agentId: 'accepted-worker', detail: { status: 'completed', result: {
      id: 'accepted-worker', task: 'Inspect delegated evidence', status: 'done', result: 'Verified completion', toolEvents: [],
    } } },
  });
  const call: ToolCallEvent = {
    callId: 'accepted-spawn', toolName: 'spawn_subagent', status: 'done',
    arguments: JSON.stringify({ task: 'Inspect delegated evidence' }), argsStatus: 'done', argsBytes: 0,
    activityEvents, artifacts: { kind: 'subagent_result', id: 'accepted-worker', task: 'Inspect delegated evidence',
      status: 'queued', result: '', toolEvents: [], lifecycleTools: { observe: 'observe_subagent', cancel: 'cancel_subagent' } },
  };
  const historicalMessages: ConversationMessage[] = mode === 'historical' ? [{
    id: 'historical-spawn', conversationId: 'parent-fixture', role: 'assistant', content: '', toolCallId: null,
    toolCalls: [{ id: call.callId, name: call.toolName, arguments: call.arguments }], artifacts: null,
    tokenCount: 0, createdAt: '2026-09-29T00:00:00Z', sortOrder: 0, thinking: 'Delegating inspection', imageAttachments: null,
  }, {
    id: 'historical-result', conversationId: 'parent-fixture', role: 'tool', content: 'Worker accepted.', toolCallId: call.callId,
    toolCalls: [], artifacts: call.artifacts ?? null, tokenCount: 0, createdAt: '2026-09-29T00:00:01Z', sortOrder: 1,
    thinking: null, imageAttachments: null,
  }] : [];
  const messages: ConversationMessage[] = [{
    id: 'system-context', conversationId: 'parent-fixture', role: 'system', content: 'Inspect delegated evidence.',
    toolCallId: null, toolCalls: [], artifacts: null, tokenCount: 0, createdAt: '2026-09-29T00:00:00Z', sortOrder: -1,
    thinking: null, imageAttachments: null,
  }, ...historicalMessages];
  return <div className="flex h-screen flex-col bg-surface-0 text-text-primary" data-testid="subagent-parent-fixture">
    <div className="flex shrink-0 gap-4 p-4">
      <button onClick={() => setPhase('running')}>Admit worker</button>
      <button onClick={() => setPhase('cancelling')}>Cancel worker</button>
      <button onClick={() => setMode('settled')}>Finish parent run</button>
      <button onClick={() => setMode('historical')}>Restore saved history</button>
      <button onClick={() => { setPhase('completed'); setMode('settled'); }}>Receive terminal evidence</button>
    </div>
    <div className="min-h-0 flex-1">
      <ChatMessages messages={messages} turns={[]} streamText="" streamRounds={[]}
        traceEvents={mode === 'historical' ? [] : [{ id: 'accepted-spawn-trace', kind: 'tool', toolCall: call }]}
        thinkingText="" isThinking={parentActive} toolCalls={mode === 'historical' ? [] : [call]} isStreaming={parentActive} />
      <TaskBoard messages={messages} toolCalls={mode === 'historical' ? [] : [call]} isStreaming={parentActive} />
    </div>
  </div>;
}

export function renderSubagentParentLifecycle() {
  const host = document.createElement('div');
  host.style.cssText = 'position:fixed;inset:0;z-index:1000';
  document.body.append(host);
  createRoot(host).render(<I18nProvider><Fixture /></I18nProvider>);
}
