import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { InteractionTrace, type InteractionStep } from '../../src/components/chat/InteractionTrace';
import { streamStore } from '../../src/lib/streamStore';
import type { AgentRunEvent, ToolRunItem } from '../../src/types/conversation';

const conversationId = 'interaction-fixture';
const canonical = (evidence: string, condition = 'notRequested') => ({ data: { interaction: { delivery: 'inputSent', evidence, condition, observationConsumed: true } } });
const steps: InteractionStep[] = [
  { kind: 'tool', id: 'observe-browser', trace: true, toolCall: { callId: 'observe-browser', toolName: 'browser_session', status: 'running', arguments: '{"action":"observe","tabId":"tab"}', argsStatus: 'ready', argsBytes: 0 } },
  { kind: 'tool', id: 'click-desktop', trace: true, toolCall: { callId: 'click-desktop', toolName: 'computer_control', status: 'done', arguments: '{"action":"click","window_id":42,"x":211,"y":433}', artifacts: canonical('unavailable'), content: 'Input sent; post-action observation unavailable.', argsStatus: 'done', argsBytes: 0 } },
  { kind: 'tool', id: 'observe-desktop', trace: true, toolCall: { callId: 'observe-desktop', toolName: 'computer_observe', status: 'done', arguments: '{"action":"wait_for_change","window_id":42}', artifacts: canonical('unchanged'), content: 'No change observed.', argsStatus: 'done', argsBytes: 0 } },
];

export function renderComputerUseTrace() {
  localStorage.setItem('nexa-locale', 'en');
  streamStore.startStream(conversationId);
  let sequence = 0;
  const runs: ToolRunItem[] = steps.map(step => ({
    callId: step.toolCall.callId, toolName: step.toolCall.toolName,
    arguments: step.toolCall.arguments, artifacts: step.toolCall.artifacts, content: step.toolCall.content,
    status: step.toolCall.status === 'running' ? 'running' : 'completed', renderKind: 'generic',
    owner: { id: 'fixture', name: 'Fixture', capability: 'test', description: '' },
    capabilities: { inputStreaming: 'none', renderKind: 'generic', readOnly: step.toolCall.toolName !== 'computer_control', destructive: false, concurrencySafe: true, interruptBehavior: 'cancel', resourceKeys: [] },
  }));
  const dispatch = (run: ToolRunItem, persistence: AgentRunEvent['persistence'] = 'durable') => {
    const event: AgentRunEvent = {
      version: 2, runId: 'interaction-run', turnId: 'interaction-turn', eventSeq: ++sequence,
      kind: run.status === 'running' ? 'toolStarted' : 'toolCompleted', phase: 'tooling', visibility: 'user', persistence, displayKind: 'tool', importance: 'normal', label: run.toolName, createdAt: new Date().toISOString(), payload: { run },
    };
    streamStore.dispatch(conversationId, { conversationId, runEvent: event });
  };
  runs.forEach(run => dispatch(run));
  // Preserve the exact occurrence identity that the real timeline passes down.
  steps.forEach(step => { step.toolCall = streamStore.selectTool(conversationId, step.toolCall.callId)!; });
  const canvas = document.createElement('canvas'); canvas.width = 240; canvas.height = 100;
  const context = canvas.getContext('2d')!;
  context.fillStyle = '#143a32'; context.fillRect(0, 0, 240, 100);
  const visual = { kind: 'toolVisualEvidence', persistence: 'currentTurnOnly', evidence: { name: 'Browser capture', mimeType: 'image/png', base64: canvas.toDataURL('image/png').split(',')[1] } };
  Object.assign(window, { __COMPUTER_TRACE_TEST__: {
    finishFirst() {
      const run: ToolRunItem = { ...runs[0], status: 'completed', artifacts: { artifacts: { kind: 'browserObservation' } }, content: 'Observed browser tab.' };
      dispatch(run);
      dispatch({ ...run, artifacts: visual }, 'ephemeral');
      dispatch(run); // Reconciliation must preserve one capture on one card.
    },
    verify() { dispatch({ ...runs[1], artifacts: canonical('unchanged', 'matched') }); },
    fail() { dispatch({ ...runs[0], status: 'failed', isError: true, artifacts: {}, content: 'Fresh observation required' }); },
  } });
  const root = document.createElement('div'); document.body.append(root);
  createRoot(root).render(<I18nProvider><main className="p-4"><InteractionTrace steps={steps} conversationId={conversationId} parentRunActive /></main></I18nProvider>);
}
