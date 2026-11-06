import { useState, type ReactNode } from 'react';
import { Check, ChevronDown, CircleAlert, Loader2, Terminal } from 'lucide-react';
import { StreamingMarkdown } from '../../components/chat/StreamingMarkdown';
import { useTranslation } from '../../i18n';
import { stripFollowupBlocks } from '../../lib/followupSuggestions';

export type RemoteTimelineItem = {
  id: string;
  sequence: number;
  onMore?: () => void;
} & ({
  kind: 'answer' | 'thinking';
  text: string;
} | {
  kind: 'tool';
  tool: { toolName: string; status: string; arguments?: string; content?: string; progressNote?: string };
});

function sameItem(a: RemoteTimelineItem, b: RemoteTimelineItem) {
  if (a.kind !== b.kind) return false;
  if (a.kind === 'tool' || b.kind === 'tool') return a.id === b.id;
  return a.text === b.text || Boolean(a.onMore && b.text.startsWith(a.text));
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
      merged.push(...live.slice(cursor, match), { ...live[match], onMore: item.onMore });
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

export function RemoteRunTimeline({ saved, live, running, finalText, children }: {
  saved: RemoteTimelineItem[]; live: RemoteTimelineItem[]; running: boolean;
  finalText?: string; children?: ReactNode;
}) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const last = live[live.length - 1];
  const hasStreamAnswer = last?.kind === 'answer' && finalText === undefined && !saved.some(item => sameItem(item, last));
  const canonicalReplacesLast = last?.kind === 'answer' && finalText !== undefined
    && (last.text.startsWith(finalText) || finalText.startsWith(last.text));
  const activity = mergeActivity(saved, hasStreamAnswer || canonicalReplacesLast ? live.slice(0, -1) : live);
  return <div className="remote-run-timeline" data-testid={live.length ? 'remote-run-timeline' : undefined}>
    {!!activity.length && <details className="remote-activity" data-testid="remote-turn-activity" onToggle={event => setExpanded(event.currentTarget.open)}>
      <summary className="cursor-pointer text-xs text-text-secondary">{t('remote.activity')}</summary>
      {expanded && <div className="remote-run-timeline mt-4">
        {activity.map(item => item.kind === 'tool' ? <ToolStep key={item.id} tool={item.tool} running={running}/> : (
          <section key={item.id} data-timeline-kind={item.kind} className="min-w-0 break-words">
            <StreamingMarkdown content={item.kind === 'answer' ? stripFollowupBlocks(item.text) : item.text} isStreaming={running} reduceMotion />
            {item.onMore && <button onClick={item.onMore}>{t('remote.moreText')}</button>}
          </section>
        ))}
      </div>}
    </details>}
    {children}
    {hasStreamAnswer && last.kind === 'answer' && <article className="remote-answer" data-timeline-kind="answer">
      <p className="mb-3 text-xs font-semibold text-accent">Nexa</p>
      <StreamingMarkdown content={stripFollowupBlocks(last.text)} isStreaming={running} reduceMotion />
    </article>}
  </div>;
}
