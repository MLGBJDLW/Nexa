import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ChatMessages } from '../../src/features/chat/ChatMessages';
import { RollingCount } from '../../src/components/chat/DiffStatsTicker';
import { TurnFileChanges } from '../../src/components/chat/TurnFileChanges';
import { I18nProvider } from '../../src/i18n';
import { SpeechPlaybackProvider } from '../../src/features/voice/SpeechPlaybackProvider';
import { streamStore } from '../../src/lib/streamStore';
import { useAgentStream } from '../../src/lib/useAgentStream';
import type { AgentRunEvent } from '../../src/types/conversation';

const id = 'file-accumulation-fixture';
let sequence = 0;
let call = 0;
function emit(kind: AgentRunEvent['kind'], payload: object) {
  streamStore.dispatch(id, { conversationId: id, runEvent: {
    version: 2, runId: id, turnId: id, eventSeq: ++sequence,
    kind, phase: 'tooling', visibility: 'user', persistence: 'durable',
    displayKind: 'tool', importance: 'normal', label: kind, payload,
  } as AgentRunEvent });
}

function Probe() {
  const stream = useAgentStream(id);
  const [count, setCount] = useState(9);
  function edit(path = 'src/notes.md', failed = false) {
    if (!call) streamStore.startStream(id);
    const index = ++call;
    const toolName = index === 1 ? 'create_file' : 'edit_file';
    const run = {
      callId: `file-${index}`, toolName, status: 'running', renderKind: 'fileChange',
      arguments: JSON.stringify({ path, content: 'text', old_string: 'before', new_string: 'after' }),
    };
    emit('toolStarted', { run });
    emit('toolCompleted', { run: { ...run, status: failed ? 'failed' : 'completed', isError: failed,
      content: failed ? 'Expected text was not found' : 'File saved',
      artifacts: failed ? undefined : { diffStats: { additions: index === 1 ? 9 : 1, deletions: index === 1 ? 0 : 1, replacements: 1, filesChanged: 1, paths: [path] } },
    } });
    emit('outputDelta', { channel: 'answer', blockId: `reply-${index}`, offset: 0, delta: `Verified step ${index}.` });
  }
  return <main className="flex h-screen flex-col">
    <div className="flex gap-3">
      <button onClick={() => edit()}>Edit notes</button>
      <button onClick={() => edit('other/notes.md')}>Edit other file</button>
      <button onClick={() => edit('src/notes.md', true)}>Fail edit</button>
      <button onClick={() => setCount(value => value === 9 ? 10 : value === 10 ? 100 : 9)}>Change digits</button>
      <span data-testid="counter-probe"><RollingCount value={count} /></span>
    </div>
    <div className="p-6" data-testid="capsule-probe"><TurnFileChanges conversationId={id} docked summary={{
      turnId: id, revision: count, additions: count, deletions: count - 4, pending: false, partial: false, unknownFiles: 0,
      files: [{ path: 'src/notes.md', absolutePath: 'D:/fixture/src/notes.md', operation: 'edit', additions: count, deletions: count - 4, contentKind: 'text', partial: false, revision: count }],
    }} /></div>
    <ChatMessages messages={[]} turns={[]} {...stream} />
  </main>;
}

export function renderFileToolAccumulation() {
  localStorage.setItem('nexa-locale', 'en');
  const root = document.createElement('div');
  document.body.replaceChildren(root);
  createRoot(root).render(<I18nProvider><SpeechPlaybackProvider><Probe /></SpeechPlaybackProvider></I18nProvider>);
}
