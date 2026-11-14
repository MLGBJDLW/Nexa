import { Fragment, useState, type ReactNode } from 'react';
import { Check, ChevronDown, CircleAlert, Loader2, Terminal } from 'lucide-react';
import { StreamingMarkdown } from '../../components/chat/StreamingMarkdown';
import { useTranslation } from '../../i18n';
import { stripFollowupBlocks } from '../../lib/followupSuggestions';
import { hasInlineChatPreview } from '../../lib/markdownCodeProtection';
import { compactActivityLabel, thinkingSummary } from '../../lib/thinkingSummary';
import { completeReplayText } from './remoteHistory';

export type RemoteTimelineItem = {
  id: string;
  sequence: number;
  onMore?: () => void;
  totalChars?: number;
  truncated?: boolean;
} & ({
  kind: 'answer';
  text: string;
} | {
  kind: 'thinking';
  text: string;
} | {
  kind: 'tool';
  tool: { toolName: string; status: string; arguments?: string; content?: string; progressNote?: string };
});

function sameItem(a: RemoteTimelineItem, b: RemoteTimelineItem) {
  if (a.kind !== b.kind) return false;
  if (a.kind === 'tool' || b.kind === 'tool') return a.id === b.id;
  return a.text === b.text || Boolean(a.onMore && b.text.startsWith(a.text))
    || Boolean(b.truncated && a.text.startsWith(b.text));
}

// Match persisted steps to replayed steps in order (identical repeated calls or
// text still have separate owners). The live tool carries its latest details.
function mergeActivity(saved: RemoteTimelineItem[], live: RemoteTimelineItem[]) {
  const merged: RemoteTimelineItem[] = [];
  let cursor = 0;
  for (const item of saved) {
    const match = live.findIndex((candidate, index) => index >= cursor && sameItem(item, candidate));
    if (match < 0) {
      merged.push(item);
    } else {
      const replay = live[match];
      const whole = item.onMore && item.kind !== 'tool' && replay.kind !== 'tool' && item.totalChars !== undefined
        && completeReplayText(item.text, item.totalChars, replay.text);
      const longerPage = replay.truncated && item.kind !== 'tool' && replay.kind !== 'tool' && item.text.length >= replay.text.length;
      const current = longerPage ? { ...item, id: replay.id } : item.onMore && !whole ? { ...replay, onMore: item.onMore } : replay;
      merged.push(...live.slice(cursor, match), current);
      cursor = match + 1;
    }
  }
  return [...merged, ...live.slice(cursor)];
}

function ToolStep({ tool, running }: { tool: Extract<RemoteTimelineItem, { kind: 'tool' }>['tool']; running: boolean }) {
  const details = Boolean(tool.progressNote || tool.arguments || tool.content);
  const label = <>
    {['failed', 'timedOut', 'declined', 'cancelled'].includes(tool.status)
      ? <CircleAlert size={15} className="shrink-0 text-danger" />
      : tool.status === 'completed' ? <Check size={15} className="shrink-0 text-success" />
        : running && tool.status !== 'recorded' ? <Loader2 size={15} className="shrink-0 animate-spin text-accent" />
          : <Terminal size={15} className="shrink-0 text-text-tertiary" />}
    <span className="min-w-0 flex-1 break-words font-mono text-xs">{tool.toolName}</span>
  </>;
  if (!details) return <div className="remote-tool flex items-center gap-2.5" data-timeline-kind="tool">{label}</div>;
  return <details className="remote-tool" data-timeline-kind="tool">
    <summary className="flex cursor-pointer list-none items-center gap-2.5">
      {label}<ChevronDown size={14} className="shrink-0 text-text-tertiary" />
    </summary>
    {tool.progressNote && <p className="mt-3 break-words text-xs leading-5 text-text-secondary">{tool.progressNote}</p>}
    {tool.arguments && <pre className="mt-3 max-h-40 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-surface-2 p-3 text-xs">{tool.arguments}</pre>}
    {tool.content && <div className="mt-3 max-h-80 overflow-auto"><StreamingMarkdown content={tool.content} isStreaming={false} reduceMotion /></div>}
  </details>;
}

function activitySummary(items: RemoteTimelineItem[]): string {
  for (let index = items.length - 1; index >= 0; index--) {
    const item = items[index];
    if (item.kind === 'tool') {
      const previous = items[index - 1];
      const note = compactActivityLabel(item.tool.progressNote ?? '')
        || (previous?.kind === 'thinking' ? thinkingSummary(previous.text) : '');
      return note || compactActivityLabel(item.tool.toolName.replace(/_/g, ' '));
    }
    if (item.kind === 'thinking') {
      const summary = thinkingSummary(item.text);
      if (summary) return summary;
    }
  }
  return '';
}

function ActivityGroup({ items, running }: { items: RemoteTimelineItem[]; running: boolean }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const summary = !expanded && activitySummary(items);
  return <details className="remote-activity" data-testid="remote-turn-activity" onToggle={event => setExpanded(event.currentTarget.open)}>
    <summary className="flex cursor-pointer list-none items-center gap-1.5 text-xs text-text-secondary">
      <ChevronDown aria-hidden="true" size={14} className={expanded ? 'shrink-0' : 'shrink-0 -rotate-90'} />
      <span className="shrink-0">{t('remote.activity')}</span>
      {summary && <span title={summary} className="min-w-0 truncate text-text-tertiary">· {summary}</span>}
    </summary>
    {expanded && <div className="remote-run-timeline mt-4">
      {items.map(item => item.kind === 'tool' ? <ToolStep key={item.id} tool={item.tool} running={running}/> : (
        <section key={item.id} data-timeline-kind={item.kind} className="min-w-0 break-words">
          <StreamingMarkdown content={item.kind === 'answer' ? stripFollowupBlocks(item.text) : item.text} isStreaming={running} reduceMotion />
          {item.onMore && <button onClick={item.onMore}>{t('remote.moreText')}</button>}
        </section>
      ))}
    </div>}
  </details>;
}

function TimelineAnswer({ item, streaming }: { item: Extract<RemoteTimelineItem, { kind: 'answer' }>; streaming: boolean }) {
  const { t } = useTranslation();
  return <article className="remote-answer" data-timeline-kind="answer">
    <p className="mb-3 text-xs font-semibold text-accent">Nexa</p>
    <StreamingMarkdown content={stripFollowupBlocks(item.text)} isStreaming={streaming} partial={!!(item.onMore || item.truncated)} reduceMotion />
    {item.onMore && <button onClick={item.onMore}>{t('remote.moreText')}</button>}
  </article>;
}

export function RemoteRunTimeline({ saved, live, running, finalText, children }: {
  saved: RemoteTimelineItem[]; live: RemoteTimelineItem[]; running: boolean;
  finalText?: string; children?: ReactNode;
}) {
  const last = live[live.length - 1];
  const hasStreamAnswer = last?.kind === 'answer' && finalText === undefined && !saved.some(item => sameItem(item, last));
  const canonicalReplacesLast = last?.kind === 'answer' && finalText !== undefined
    && (last.text.startsWith(finalText) || finalText.startsWith(last.text));
  const activity = mergeActivity(saved, hasStreamAnswer || canonicalReplacesLast ? live.slice(0, -1) : live);
  const rows: ReactNode[] = [];
  let group: RemoteTimelineItem[] = [];
  function flushGroup() {
    if (!group.length) return;
    rows.push(<ActivityGroup key={'activity:' + group[0].id} items={group} running={running} />);
    group = [];
  }
  for (const item of activity) {
    if (item.kind === 'answer' && hasInlineChatPreview(item)) {
      flushGroup();
      rows.push(<TimelineAnswer key={item.id} item={item} streaming={running && last?.id === item.id} />);
    } else {
      group.push(item);
    }
  }
  flushGroup();
  rows.push(<Fragment key="messages">{children}</Fragment>);
  if (hasStreamAnswer) rows.push(<TimelineAnswer key={last.id} item={last} streaming={running} />);
  return <div className="remote-run-timeline" data-testid={live.length ? 'remote-run-timeline' : undefined}>{rows}</div>;
}
