import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { ToolCallCard } from '../../src/components/chat/ToolCallCard';
import { createDefaultState } from '../../src/lib/streaming/state';
import { applyToolRunEvent } from '../../src/lib/streaming/toolProjection';
import type { ToolRunItem } from '../../src/types/conversation';

const run = { callId: 'server', toolName: 'run_shell', status: 'running', arguments: '{"program":"npm","args":["run","dev"]}' } as ToolRunItem;
const event = (seq: number, kind: string, payload: Record<string, unknown>) => ({ activityId: 'server', seq, kind, timestamp: new Date().toISOString(), payload });
const initial = () => {
  const state = createDefaultState();
  applyToolRunEvent(state, { ...run, artifacts: { activity: event(1, 'started', { state: 'running' }) } });
  applyToolRunEvent(state, { ...run, artifacts: { activity: event(2, 'stdout_chunk', { data: 'Starting development server...\n' }) } });
  applyToolRunEvent(state, { ...run, artifacts: { activity: event(3, 'stderr_chunk', { data: 'Compiling components...\n' }) } });
  return state;
};
function Fixture() {
  const [state, setState] = useState(initial);
  const call = state.toolCalls[0];
  const ready = () => setState(previous => {
    const next = { ...previous };
    applyToolRunEvent(next, { ...run, status: 'completed', artifacts: { kind: 'managedService', activityId: 'server', serviceId: 'server', cursor: 4, status: 'ready', readyUrl: 'http://127.0.0.1:5173/', program: 'npm', stdoutTail: 'Development server ready\n', stderrTail: '' } });
    return next;
  });
  return <main className="max-w-full p-5">
    <ToolCallCard toolName={call.toolName} arguments={call.arguments} status={call.status} artifacts={call.artifacts} activityEvents={call.activityEvents} trace />
    <button onClick={ready}>Report ready</button>
  </main>;
}
createRoot(document.getElementById('root')!).render(<I18nProvider><Fixture /></I18nProvider>);
