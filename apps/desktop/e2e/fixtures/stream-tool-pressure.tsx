import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { useAgentStream } from '../../src/lib/useAgentStream';
import { streamStore } from '../../src/lib/streamStore';
import { ChatMessages } from '../../src/features/chat/ChatMessages';
import { I18nProvider } from '../../src/i18n';
import { SpeechPlaybackProvider } from '../../src/features/voice/SpeechPlaybackProvider';
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
  return <main data-testid="pressure-probe" className="flex h-screen flex-col">
    <button onClick={() => {
      setRunning(true);
      streamStore.startStream(conversationId);
      let count = 0;
      const firstReply = 'Before tool reply. ';
      const firstThought = 'Before the browser starts, inspect the current page. ';
      emit('outputDelta', { channel: 'answer', blockId: 'answer', offset: 0, delta: firstReply });
      emit('outputDelta', { channel: 'thinking', blockId: 'thought', offset: 0, delta: firstThought });
      let answerOffset = firstReply.length;
      let thinkingOffset = firstThought.length;
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
    <ChatMessages messages={[]} turns={[]} {...stream} />
  </main>;
}

export function renderStreamToolPressure() {
  localStorage.setItem('nexa-locale', 'en');
  const root = document.createElement('div'); document.body.replaceChildren(root);
  createRoot(root).render(<I18nProvider><SpeechPlaybackProvider><Probe /></SpeechPlaybackProvider></I18nProvider>);
}
