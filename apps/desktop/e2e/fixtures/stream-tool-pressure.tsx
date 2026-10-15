import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { useAgentStream } from '../../src/lib/useAgentStream';
import { streamStore } from '../../src/lib/streamStore';
import type { AgentRunEvent } from '../../src/types/conversation';

const conversationId = 'tool-pressure';
let sequence = 0;
function emit(kind: AgentRunEvent['kind'], payload: object) {
  streamStore.dispatch(conversationId, { conversationId, runEvent: {
    version: 2, runId: 'pressure-run', turnId: 'pressure-turn', eventSeq: ++sequence,
    kind, phase: 'tooling', visibility: 'user', persistence: 'durable',
    displayKind: 'tool', importance: 'normal', label: kind, createdAt: new Date().toISOString(), payload,
  } as AgentRunEvent });
}

function Probe() {
  const stream = useAgentStream(conversationId);
  const [running, setRunning] = useState(false);
  return <main data-testid="pressure-probe">
    <button onClick={() => {
      setRunning(true);
      streamStore.startStream(conversationId);
      let count = 0;
      let answerOffset = 0;
      let thinkingOffset = 0;
      const run = { callId: 'browser', toolName: 'browser_session', status: 'running', arguments: '{}' };
      emit('toolStarted', { run });
      const timer = setInterval(() => {
        count++;
        if (count === 2) emit('toolPreparing', { run: { ...run, callId: 'second-browser', status: 'preparing' } });
        const thinking = `Thought ${count}. `;
        const answer = `Answer ${count}. `;
        emit('outputDelta', { channel: 'thinking', blockId: 'thought', offset: thinkingOffset, delta: thinking });
        thinkingOffset += thinking.length;
        emit('outputDelta', { channel: 'answer', blockId: 'answer', offset: answerOffset, delta: answer });
        answerOffset += answer.length;
        emit('toolProgress', { run: { ...run, progressNote: `Browser frame ${count}` } });
        document.documentElement.dataset.pressureEvents = String(count);
      }, 16);
      Object.assign(window, { __stopToolPressure: () => { clearInterval(timer); setRunning(false); } });
    }}>Start long browser tool</button>
    <output data-testid="pressure-running">{String(running)}</output>
    <output data-testid="pressure-answer">{stream.streamText}</output>
    <output data-testid="pressure-thinking">{stream.traceEvents.filter(event => event.kind === 'thinking').map(event => event.text).join('')}</output>
  </main>;
}

export function renderStreamToolPressure() {
  const root = document.createElement('div'); document.body.replaceChildren(root);
  createRoot(root).render(<Probe />);
}
