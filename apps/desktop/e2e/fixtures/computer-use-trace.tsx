import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { ComputerUseTrace, type ComputerUseStep } from '../../src/components/chat/ComputerUseTrace';
import { streamStore } from '../../src/lib/streamStore';
import type { AgentRunEvent } from '../../src/types/conversation';

const conversationId = 'computer-use-fixture';
const steps: ComputerUseStep[] = [
  { kind: 'tool', id: 'observe-1', trace: true, toolCall: { callId: 'observe-1', toolName: 'computer_observe', status: 'done', arguments: '{"action":"capture_window","window_id":42}', content: 'Observed the selected window' } },
  { kind: 'tool', id: 'click-1', trace: true, toolCall: { callId: 'click-1', toolName: 'computer_control', status: 'done', arguments: '{"action":"click","window_id":42,"x":211,"y":433}', content: 'Clicked and observed the result' } },
  { kind: 'tool', id: 'observe-2', trace: true, toolCall: { callId: 'observe-2', toolName: 'computer_observe', status: 'running', arguments: '{"action":"wait_for_change","window_id":42}' } },
];

export function renderComputerUseTrace() {
  localStorage.setItem('nexa-locale', 'en');
  streamStore.startStream(conversationId);
  const event: AgentRunEvent = {
    version: 2, runId: 'computer-run', turnId: 'computer-turn', eventSeq: 1,
    kind: 'toolStarted', phase: 'tooling', visibility: 'user', persistence: 'durable', displayKind: 'tool', importance: 'normal', label: 'computer_observe', createdAt: new Date().toISOString(),
    payload: { run: { callId: 'observe-2', toolName: 'computer_observe', status: 'running', arguments: steps[2].toolCall.arguments, renderKind: 'generic', owner: { id: 'fixture', name: 'Fixture', capability: 'test', description: '' }, capabilities: { inputStreaming: 'none', renderKind: 'generic', readOnly: true, destructive: false, concurrencySafe: true, interruptBehavior: 'cancel', resourceKeys: [] } } },
  };
  streamStore.dispatch(conversationId, { conversationId, runEvent: event });
  // Bind the same immutable occurrence used by the real timeline; an unrelated
  // object with a reused callId must deliberately not adopt live state.
  steps[2].toolCall = streamStore.selectTool(conversationId, 'observe-2') ?? steps[2].toolCall;
  Object.assign(window, { __COMPUTER_TRACE_TEST__: {
    fail() {
      const completed: AgentRunEvent = {
        ...event, eventSeq: 2, kind: 'toolCompleted',
        payload: { run: { ...(event.payload as { run: object }).run, status: 'failed', isError: true, content: 'Fresh observation required' } },
      };
      streamStore.dispatch(conversationId, { conversationId, runEvent: completed });
    },
  } });
  const root = document.createElement('div'); document.body.append(root);
  createRoot(root).render(<I18nProvider><main className="p-4"><ComputerUseTrace steps={steps} conversationId={conversationId} parentRunActive /></main></I18nProvider>);
}
