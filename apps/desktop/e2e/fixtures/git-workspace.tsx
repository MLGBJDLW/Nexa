import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { TaskBoard } from '../../src/components/chat/TaskBoard';
import type { ToolCallEvent } from '../../src/lib/streaming/protocol';
import '../../src/index.css';

const messages = [];
const calls: ToolCallEvent[] = [{ callId: 'plan', toolName: 'update_plan', status: 'done', arguments: '{}', argsStatus: 'done', argsBytes: 2,
  artifacts: { kind: 'plan', title: 'Workspace audit', steps: [{ id: 'one', title: 'Verify the working tree', status: 'in_progress' }] },
}];

function Fixture() {
  const [revision, setRevision] = useState(0);
  const [conversation, setConversation] = useState('git-a');
  return <div className="relative h-screen bg-surface-0 text-text-primary">
    <div className="absolute bottom-4 left-4 flex gap-4">
      <button onClick={() => setRevision(value => value + 1)}>Tool completed</button>
      <button onClick={() => setConversation('git-b')}>Switch conversation</button>
    </div>
    <TaskBoard conversationId={conversation} isStreaming sourceRevision={String(revision)} messages={messages} toolCalls={calls} />
  </div>;
}

createRoot(document.getElementById('root')!).render(<I18nProvider><Fixture /></I18nProvider>);
