import { createRoot } from 'react-dom/client';
import { ChatMessages } from '../../src/features/chat/ChatMessages';
import { I18nProvider } from '../../src/i18n';
import { SpeechPlaybackProvider } from '../../src/features/voice/SpeechPlaybackProvider';
import type { ConversationMessage, ConversationTurn } from '../../src/types/conversation';

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
