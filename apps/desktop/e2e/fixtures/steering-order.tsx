import { createRoot } from 'react-dom/client';
import { useState } from 'react';
import { ChatMessages } from '../../src/features/chat/ChatMessages';
import { I18nProvider } from '../../src/i18n';
import { SpeechPlaybackProvider } from '../../src/features/voice/SpeechPlaybackProvider';
import type { ConversationMessage, ConversationTurn } from '../../src/types/conversation';
import { createDefaultState } from '../../src/lib/streaming/state';
import { applyStreamBlockDelta } from '../../src/lib/streaming/blockProjection';
import { applyDoneEvent, applyStatusEvent } from '../../src/lib/streaming/liveProjection';
import { applyStreamResetProjection } from '../../src/lib/streaming/terminalProjection';

export function renderSteeringHistory() {
  const host = document.createElement('div');
  host.style.cssText = 'position:fixed;inset:0;z-index:1000;display:flex;background:white';
  document.body.appendChild(host);
  const messages: ConversationMessage[] = [
    ['user', 'Original request'], ['assistant', 'Work before steering'],
    ['user', 'First steering at its insertion point'], ['assistant', 'Work between steering messages'],
    ['user', 'Second steering at its insertion point'], ['assistant', 'Final summary after both corrections'],
  ].map(([role, content], index) => ({
    id: `history-${index}`, conversationId: 'history-order', role: role as ConversationMessage['role'], content,
    sortOrder: index, createdAt: '2026-09-14 08:00:00', toolCalls: [], toolCallId: null,
    tokenCount: 0, thinking: null, imageAttachments: null,
    artifacts: index === 2 || index === 4 ? { kind: 'steering' } : null,
  }));
  const turn = { id: 'ordered-turn', userMessageId: 'history-0', assistantMessageId: 'history-5', conversationId: 'history-order', status: 'completed', trace: null } as ConversationTurn;
  createRoot(host).render(<I18nProvider><SpeechPlaybackProvider><ChatMessages messages={messages} turns={[turn]} streamText="" streamRounds={[]} traceEvents={[]} thinkingText="" isThinking={false} toolCalls={[]} isStreaming={false} /></SpeechPlaybackProvider></I18nProvider>);
}

export function renderSteeringTerminalHandoff(finalDelta: boolean) {
  const host = document.createElement('div');
  host.style.cssText = 'position:fixed;inset:0;z-index:1000;display:flex;flex-direction:column;background:white';
  document.body.appendChild(host);
  const messages: ConversationMessage[] = [
    ['user', 'Original request'], ['assistant', 'Earlier progress'],
    ['user', 'I was only asking'], ['assistant', 'Final summary'],
  ].map(([role, content], index) => ({
    id: `handoff-${index}`, conversationId: 'handoff', role: role as ConversationMessage['role'], content,
    sortOrder: index, createdAt: '2026-09-20 08:00:00', toolCalls: [], toolCallId: null,
    tokenCount: 0, thinking: index === 3 ? 'Review the correction' : null, imageAttachments: null,
    artifacts: index === 2 ? { kind: 'steering' } : null,
  }));
  const turn = { id: 'handoff-turn', userMessageId: 'handoff-0', assistantMessageId: 'handoff-3', conversationId: 'handoff', status: 'success', trace: null } as ConversationTurn;
  function Handoff() {
    const [phase, setPhase] = useState<'live' | 'done' | 'persisted'>('live');
    const stream = createDefaultState();
    if (phase !== 'persisted') {
      stream.isStreaming = true;
      applyStreamBlockDelta(stream, 'answer', 'earlier', 0, 'Earlier progress');
      applyStatusEvent(stream, 'I was only asking', 'muted', 'user', 'steering');
      applyStreamResetProjection(stream, 'steering_restart');
      applyStreamBlockDelta(stream, 'thinking', 'thinking-after-steering', 0, 'Review the correction');
      if (phase === 'done') {
        if (finalDelta) applyStreamBlockDelta(stream, 'answer', 'final', 0, 'Final');
        applyDoneEvent(stream, { status: 'completed', message: messages[3] });
      }
    }
    return <>
      <button onClick={() => setPhase('done')}>Complete handoff</button>
      <button onClick={() => setPhase('persisted')}>Load durable history</button>
      <ChatMessages messages={phase === 'persisted' ? messages : messages.slice(0, 1)} turns={phase === 'persisted' ? [turn] : []} {...stream} />
    </>;
  }
  createRoot(host).render(<I18nProvider><SpeechPlaybackProvider><Handoff /></SpeechPlaybackProvider></I18nProvider>);
}
