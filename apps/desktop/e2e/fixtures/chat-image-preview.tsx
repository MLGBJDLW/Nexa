import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { StreamingMarkdown } from '../../src/components/chat/StreamingMarkdown';
import { MessageBubble } from '../../src/components/chat/MessageBubble';
import { ToolCallCard } from '../../src/components/chat/ToolCallCard';
import { applyToolRunEvent } from '../../src/lib/streaming/toolProjection';
import { createDefaultState } from '../../src/lib/streaming/state';
import { FilePreviewContext } from '../../src/features/preview/filePreviewContext';
import { SpeechPlaybackProvider } from '../../src/features/voice/SpeechPlaybackProvider';
import { useChatSession } from '../../src/lib/useChatSession';
import { streamStore } from '../../src/lib/streamStore';
import type { AgentRunEvent, ToolRunItem, ConversationMessage } from '../../src/types/conversation';

const canvas = document.createElement('canvas');
canvas.width = 640; canvas.height = 360;
const context = canvas.getContext('2d')!;
context.fillStyle = '#162134'; context.fillRect(0, 0, 640, 360);
context.fillStyle = '#27364d'; context.fillRect(12, 12, 616, 38);
context.fillStyle = '#b7ceec'; context.font = '18px sans-serif'; context.fillText('Editor capture fixture', 24, 38);
context.fillStyle = '#4f83bd'; context.fillRect(24, 78, 184, 254);
context.fillStyle = '#d0e1f5'; context.fillRect(228, 78, 388, 254);
context.fillStyle = '#172e48'; context.font = '26px sans-serif'; context.fillText('Captured pixels', 248, 130);
const source = canvas.toDataURL('image/png');
const pixels = source.split(',')[1];
const receipt = { artifacts: { kind: 'computerObservation' }, data: { screenshotHash: 'capture-hash', window: { title: 'Editor' } }, toolOutput: { displayContent: 'Observed Editor.' } };
const completed: ToolRunItem = { callId: 'capture', toolName: 'computer_observe', status: 'completed', arguments: '{"action":"capture_window"}', content: 'Observed Editor.', artifacts: receipt,
  owner: { id: 'desktop', name: 'Desktop', capability: 'capture', description: 'Capture window' }, renderKind: 'generic',
  capabilities: { inputStreaming: 'none', renderKind: 'generic', readOnly: true, destructive: false, concurrencySafe: true, interruptBehavior: 'block', resourceKeys: [] } };
const visual = { kind: 'toolVisualEvidence', persistence: 'currentTurnOnly', evidence: { name: 'Editor screenshot', mimeType: 'image/png', base64: pixels, contentHash: 'capture-hash' } };

function Fixture() {
  const [state, setState] = useState(() => {
    const state = createDefaultState();
    applyToolRunEvent(state, completed);
    applyToolRunEvent(state, { ...completed, artifacts: visual });
    return state;
  });
  const tool = state.toolCalls[0];
  return <main className="relative min-h-screen bg-surface-0 p-6 text-text-primary">
    <div data-testid="markdown-images"><StreamingMarkdown content={`![Markdown screenshot](${source})\n\n![Local screenshot](D:/workspace/screenshot.png)`} isStreaming={false} /></div>
    <MessageBubble msg={{ id: 'upload', conversationId: 'images', role: 'user', content: 'Attached screenshot', toolCallId: null, toolCalls: [], artifacts: null, tokenCount: 0, createdAt: new Date().toISOString(), sortOrder: 0, thinking: null, imageAttachments: [{ originalName: 'Uploaded screenshot.png', mediaType: 'image/png', base64Data: pixels }] }} />
    <div data-testid="visual-tool-card"><ToolCallCard toolName={tool.toolName} args={tool.arguments} status={tool.status} content={tool.content} artifacts={tool.artifacts} compact={new URLSearchParams(location.search).has('compact')} trace={new URLSearchParams(location.search).has('trace')} /></div>
    <button onClick={() => setState(previous => { const next = { ...previous }; applyToolRunEvent(next, completed); return next; })}>Reconcile durable tool result</button>
    <ToolCallCard toolName="generate_image" args="{}" status="done" renderKind="image" artifacts={{ kind: 'generatedImage', dataUrl: source, mediaType: 'image/png', prompt: 'Generated screenshot' }} />
  </main>;
}

const sessionId = 'image-session';
const timestamp = '2026-09-22T00:00:00Z';
const conversation = { id: sessionId, title: 'Image session', provider: 'open_ai', model: 'test', systemPrompt: '', collectionContext: null, projectId: null, createdAt: timestamp, updatedAt: timestamp };
const base = { conversationId: sessionId, tokenCount: 0, createdAt: timestamp, sortOrder: 0, thinking: null, toolCallId: null, toolCalls: [], artifacts: null, imageAttachments: null };
const storedMessages: ConversationMessage[] = [
  { ...base, id: 'user', role: 'user', content: 'Capture Editor' },
  { ...base, id: 'call', role: 'assistant', content: '', toolCalls: [{ id: completed.callId, name: completed.toolName, arguments: completed.arguments! }], sortOrder: 1 },
  { ...base, id: 'result', role: 'tool', content: completed.content!, toolCallId: completed.callId, artifacts: receipt, sortOrder: 2 },
  { ...base, id: 'answer', role: 'assistant', content: 'Captured Editor.', sortOrder: 3 },
];
function dispatch(eventSeq: number, kind: AgentRunEvent['kind'], payload: Record<string, unknown>, persistence: AgentRunEvent['persistence'] = 'durable') {
  streamStore.dispatch(sessionId, { conversationId: sessionId, runEvent: { version: 2, runId: 'capture-run', turnId: 'capture-turn', eventSeq, kind, phase: kind === 'done' ? 'done' : 'tooling', label: kind, status: kind === 'done' ? 'completed' : 'running', payload, visibility: 'user', persistence, displayKind: kind === 'done' ? 'completion' : 'tool', importance: 'normal', createdAt: timestamp } });
}
function SessionFixture() {
  const session = useChatSession({ conversationId: sessionId });
  const persistedTool = session.messages.find(message => message.role === 'tool');
  const call = session.toolCalls[0];
  return <main className="bg-surface-0 p-6 text-text-primary">
    <div data-testid="session-status">{session.isStreaming ? 'streaming' : 'settled'} / {session.loadingMsgs ? 'loading' : 'loaded'}</div>
    <button onClick={() => { streamStore.startStream(sessionId); dispatch(1, 'toolCompleted', { run: completed }); dispatch(2, 'toolProgress', { run: { ...completed, artifacts: visual } }, 'ephemeral'); }}>Capture screenshot</button>
    <button onClick={() => { localStorage.setItem('image-fixture-done', 'true'); dispatch(3, 'done', { message: { content: 'Captured Editor.' } }); }}>Finish turn</button>
    <button onClick={() => void session.reloadMessages()}>Refresh stored messages</button>
    {(call || persistedTool) && <ToolCallCard toolName={completed.toolName} status="done" content={call?.content ?? persistedTool?.content} artifacts={call?.artifacts ?? persistedTool?.artifacts ?? undefined} />}
  </main>;
}

export function renderChatImages() {
  const session = new URLSearchParams(location.search).has('session');
  if (session) Object.assign(window, { __TAURI_INTERNALS__: { async invoke(command: string) {
    if (command === 'list_conversations_cmd') return [conversation];
    if (command === 'get_conversation_cmd') return [conversation, structuredClone(localStorage.getItem('image-fixture-done') ? storedMessages : storedMessages.slice(0, 1))];
    if (command.startsWith('get_conversation_turns') || command.startsWith('get_agent_task_runs') || command.startsWith('list_agent_configs')) return [];
    return null;
  } } });
  const host = document.createElement('div');
  host.style.cssText = 'position:fixed;inset:0;z-index:1000;overflow:auto';
  document.body.appendChild(host);
  createRoot(host).render(<I18nProvider><SpeechPlaybackProvider><FilePreviewContext.Provider value={{ openFilePreview() {}, openWebLink() {}, resolveFileUrl: async () => source }}>{session ? <SessionFixture /> : <Fixture />}</FilePreviewContext.Provider></SpeechPlaybackProvider></I18nProvider>);
}
