import { ArrowDown, CornerDownRight, CircleStop, Volume2 } from 'lucide-react';
import { StreamingMarkdown } from '../../components/chat/StreamingMarkdown';
import { useTranslation } from '../../i18n';
import { stripFollowupBlocks } from '../../lib/followupSuggestions';
import { remoteButton } from './remoteUi';
import { RemoteRunTimeline, type RemoteTimelineItem } from './RemoteRunTimeline';

export interface ChatMessage {
  id: string;
  role: string;
  content: string;
  totalChars: number;
  sortOrder: number;
  intermediate?: boolean;
  continuesTurn?: boolean;
  steering?: boolean;
  precedingUserId?: string;
  toolCalls?: Array<{ id: string; name: string }>;
  thinking?: string;
  thinkingTotalChars?: number;
}

function RemoteChatMessage({ message, onMore, onSpeak, speaking }: { message: ChatMessage; onMore: () => void; onSpeak?: () => void; speaking?: boolean }) {
  const { t } = useTranslation();
  if (message.steering) return <article className="remote-steering flex items-start gap-2 rounded-lg border-l-2 border-accent bg-surface-1 px-3 py-2" aria-label={t('chat.steeringMessage')}>
    <CornerDownRight size={16} className="mt-0.5 shrink-0 text-accent" aria-hidden="true"/>
    <div className="min-w-0"><small className="text-accent">{t('chat.steeringLabel')}</small><p className="whitespace-pre-wrap break-words">{message.content}</p>
      {[...message.content].length < message.totalChars && <button className={remoteButton} onClick={onMore}>{t('remote.moreText')}</button>}
    </div>
  </article>;
  return <article className={`remote-message ${message.role === 'user' ? 'remote-message-user' : 'remote-answer'}`}>
    <p className="mb-3 text-xs font-semibold text-text-tertiary">{message.role === 'user' ? t('remote.you') : 'Nexa'}</p>
    {message.role === 'user' ? <div className="whitespace-pre-wrap break-words text-sm leading-7">{message.content}</div>
      : <StreamingMarkdown content={stripFollowupBlocks(message.content)} isStreaming={false} reduceMotion />}
    {[...message.content].length < message.totalChars && <button className={`${remoteButton} mt-3`} onClick={onMore}>
      <ArrowDown size={14} />{t('remote.moreText')}
    </button>}
    {message.role === 'assistant' && onSpeak && <button className="mobile-read-aloud" aria-label={t(speaking ? 'remote.stopReading' : 'remote.readAloud')} onClick={onSpeak}>{speaking ? <CircleStop size={16}/> : <Volume2 size={16}/>}</button>}
  </article>;
}

interface PhoneTurn {
  key: string;
  promptId?: string;
  visible: ChatMessage[];
  activityIndex?: number;
  activity: RemoteTimelineItem[];
}

export function RemoteTranscript({ messages, timeline, running, finalMessageId, runUserId, runId, more, speak, speakingId }: {
  messages: ChatMessage[]; timeline: RemoteTimelineItem[]; running: boolean; finalMessageId: string | null;
  runId: string | null; runUserId: string | null; more: (message: ChatMessage, channel?: 'content' | 'thinking') => void;
  speak?: (message: ChatMessage) => void; speakingId: string | null;
}) {
  const turns: PhoneTurn[] = [];
  let turn: PhoneTurn | undefined;
  for (const message of messages) {
    // A user prompt or a new response after a visible final starts a boundary,
    // including when an older history page starts part-way through a turn.
    if (!turn || (message.role === 'user' && !message.continuesTurn)
      || (!turn.promptId && turn.visible[0]?.role === 'assistant' && !message.continuesTurn)) {
      const promptId = message.role === 'user' && !message.continuesTurn ? message.id : message.precedingUserId;
      turn = { key: promptId ?? message.id, promptId, visible: [], activity: [] };
      turns.push(turn);
    }
    if (message.role === 'user') {
      turn.visible.push(message);
      continue;
    }
    const intermediate = message.intermediate && message.id !== finalMessageId;
    for (const channel of ['thinking', 'content'] as const) {
      const text = message[channel];
      if (!text || (channel === 'content' && !intermediate)) continue;
      const length = channel === 'thinking' ? message.thinkingTotalChars : message.totalChars;
      turn.activity.push({ id: message.id + ':' + channel, sequence: message.sortOrder,
        kind: channel === 'thinking' ? 'thinking' : 'answer', text,
        onMore: length && [...text].length < length ? () => more(message, channel) : undefined });
    }
    for (const call of message.toolCalls ?? []) {
      turn.activity.push({ id: 'tool:' + call.id, sequence: message.sortOrder, kind: 'tool',
        tool: { toolName: call.name, status: 'recorded' } });
    }
    if (turn.activity.length && turn.activityIndex === undefined) turn.activityIndex = turn.visible.length;
    if (!intermediate && message.content) turn.visible.push(message);
  }

  // Old peers do not send a user anchor. Attach their replay only to a
  // known final or the last open turn, never to an unrelated prior answer.
  let liveTurn = finalMessageId ? turns.find(item => item.visible.some(message => message.id === finalMessageId)) : undefined;
  if (!liveTurn && runUserId) liveTurn = turns.find(item => item.promptId === runUserId || item.visible.some(message => message.id === runUserId));
  if (!liveTurn && !runUserId && !turn?.visible.some(message => message.role === 'assistant')) liveTurn = turn;
  if (!liveTurn && timeline.length) {
    liveTurn = { key: 'live', visible: [], activity: [] };
    turns.push(liveTurn);
  }
  const renderMessage = (message: ChatMessage) => <RemoteChatMessage key={message.id} message={message} onMore={() => more(message)}
    onSpeak={speak ? () => speak(message) : undefined} speaking={speakingId === message.id}/>;
  return <>{turns.map(item => {
    const live = item === liveTurn;
    const finalIndex = item.visible.findIndex(message => message.id === finalMessageId);
    const split = item.activityIndex ?? (finalIndex >= 0 ? finalIndex : item.visible.length);
    return <div className="remote-transcript" key={item.key}>
      {item.visible.slice(0, split).map(renderMessage)}
      {(item.activity.length > 0 || (live && timeline.length > 0)) ?
        <RemoteRunTimeline key={live ? runId : item.key} saved={item.activity} live={live ? timeline : []}
          running={live && running} finalText={item.visible[finalIndex]?.content}>
          {item.visible.slice(split).map(renderMessage)}
        </RemoteRunTimeline> : item.visible.slice(split).map(renderMessage)}
    </div>;
  })}</>;
}
