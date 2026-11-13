import { memo, useMemo } from 'react';
import { ArrowDown, CornerDownRight, CircleStop, Volume2 } from 'lucide-react';
import { StreamingMarkdown } from '../../components/chat/StreamingMarkdown';
import { useTranslation } from '../../i18n';
import { stripFollowupBlocks } from '../../lib/followupSuggestions';
import { remoteButton } from './remoteUi';
import { RemoteRunTimeline, type RemoteTimelineItem } from './RemoteRunTimeline';
import { MessageImage, type MessageAttachmentSummary, type LoadMessageAttachment } from '../../components/chat/MessageImage';

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
  attachments?: MessageAttachmentSummary[];
}

function RemoteChatMessage({ message, onMore, onSpeak, speaking, answerRef, loadAttachment }: { message: ChatMessage; onMore: () => void; onSpeak?: () => void; speaking?: boolean; answerRef?: (node: HTMLElement | null) => void; loadAttachment?: LoadMessageAttachment }) {
  const { t } = useTranslation();
  const attachments = !!message.attachments?.length && <div className="mb-2 flex max-w-full flex-wrap gap-2">{message.attachments.map(attachment =>
      attachment.mediaType?.startsWith('image/') && loadAttachment
        ? <MessageImage key={attachment.index} messageId={message.id} attachment={attachment} load={loadAttachment} />
        : <span key={attachment.index} className="rounded border border-border p-2 text-xs">{attachment.originalName}</span>,
    )}</div>;
  if (message.steering) return <article className="remote-steering flex items-start gap-2 rounded-lg border-l-2 border-accent bg-surface-1 px-3 py-2" aria-label={t('chat.steeringMessage')}>
    <CornerDownRight size={16} className="mt-0.5 shrink-0 text-accent" aria-hidden="true"/>
    <div className="min-w-0"><small className="text-accent">{t('chat.steeringLabel')}</small><p className="whitespace-pre-wrap break-words">{message.content}</p>{attachments}
      {[...message.content].length < message.totalChars && <button className={remoteButton} onClick={onMore}>{t('remote.moreText')}</button>}
    </div>
  </article>;
  return <article ref={answerRef} data-message-id={message.id} className={`remote-message ${message.role === 'user' ? 'remote-message-user' : 'remote-answer'}`}>
    <p className="mb-3 text-xs font-semibold text-text-tertiary">{message.role === 'user' ? t('remote.you') : 'Nexa'}</p>
    {attachments}
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

interface RemoteTranscriptProps {
  messages: ChatMessage[]; timeline: RemoteTimelineItem[]; running: boolean; finalMessageId: string | null;
  runId: string | null; runUserId: string | null; more: (message: ChatMessage, channel?: 'content' | 'thinking') => void;
  speak?: (message: ChatMessage) => void; speakingId: string | null; answerRef?: (node: HTMLElement | null) => void;
  loadAttachment?: LoadMessageAttachment;
}

const EMPTY_TIMELINE: RemoteTimelineItem[] = [];
const LIVE_TURN: PhoneTurn = { key: 'live', visible: [], activity: [] };

const RemoteTranscriptTurn = memo(function RemoteTranscriptTurn({ turn, timeline, running, finalMessageId, runId, more, speak, speakingId, answerRef, loadAttachment }: Omit<RemoteTranscriptProps, 'messages' | 'runUserId'> & { turn: PhoneTurn }) {
  const finalIndex = turn.visible.findIndex(message => message.id === finalMessageId);
  const split = turn.activityIndex ?? (finalIndex >= 0 ? finalIndex : turn.visible.length);
  const renderMessage = (message: ChatMessage) => <RemoteChatMessage key={message.id} message={message} onMore={() => more(message)}
    loadAttachment={loadAttachment}
    answerRef={message.id === finalMessageId ? answerRef : undefined}
    onSpeak={speak ? () => speak(message) : undefined} speaking={speakingId === message.id}/>;
  return <div className="remote-transcript">
    {turn.visible.slice(0, split).map(renderMessage)}
    {(turn.activity.length > 0 || timeline.length > 0) ?
      <RemoteRunTimeline key={runId ?? turn.key} saved={turn.activity} live={timeline}
        running={running} finalText={turn.visible[finalIndex]?.content}>
        {turn.visible.slice(split).map(renderMessage)}
      </RemoteRunTimeline> : turn.visible.slice(split).map(renderMessage)}
  </div>;
});

export const RemoteTranscript = memo(function RemoteTranscript({ messages, timeline, running, finalMessageId, runUserId, runId, more, speak, speakingId, answerRef, loadAttachment }: RemoteTranscriptProps) {
  // Streaming updates only invalidate the live turn. Loaded history keeps its
  // grouping, pagination callbacks and rendered subtrees until history changes.
  const turns = useMemo(() => {
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
    return turns;
  }, [messages, finalMessageId, more]);

  // Old peers do not send a user anchor. Attach their replay only to a
  // known final or the last open turn, never to an unrelated prior answer.
  let liveTurn = finalMessageId ? turns.find(item => item.visible.some(message => message.id === finalMessageId)) : undefined;
  if (!liveTurn && runUserId) liveTurn = turns.find(item => item.promptId === runUserId || item.visible.some(message => message.id === runUserId));
  const lastTurn = turns[turns.length - 1];
  if (!liveTurn && !runUserId && !lastTurn?.visible.some(message => message.role === 'assistant')) liveTurn = lastTurn;
  if (!liveTurn && timeline.length) liveTurn = LIVE_TURN;
  const displayedTurns = liveTurn === LIVE_TURN ? [...turns, LIVE_TURN] : turns;
  return <>{displayedTurns.map(item => {
    const live = item === liveTurn;
    return <RemoteTranscriptTurn key={item.key} turn={item} timeline={live ? timeline : EMPTY_TIMELINE}
      runId={live ? runId : null} running={live && running} finalMessageId={finalMessageId}
      more={more} speak={speak} speakingId={speakingId} answerRef={answerRef} loadAttachment={loadAttachment}/>;
  })}</>;
});
