import { Check, ChevronDown, CircleAlert, Loader2, Terminal } from 'lucide-react';
import { StreamingMarkdown } from '../../components/chat/StreamingMarkdown';
import { useTranslation } from '../../i18n';

export type RemoteTimelineItem = {
  id: string;
  sequence: number;
} & ({
  kind: 'answer' | 'thinking';
  text: string;
} | {
  kind: 'tool';
  tool: { toolName: string; status: string; arguments?: string; content?: string; progressNote?: string };
});

/** Keep model messages and tool activity in their original order on a narrow screen. */
export function RemoteRunTimeline({ items, running, settled }: {
  items: RemoteTimelineItem[];
  running: boolean;
  settled: boolean;
}) {
  const { t } = useTranslation();
  if (!items.length) return null;
  const content = <div className="remote-run-timeline" data-testid="remote-run-timeline">
    {items.map(item => item.kind === 'tool' ? (
      <details key={item.id} className="remote-tool" data-timeline-kind="tool">
        <summary className="flex cursor-pointer list-none items-center gap-2.5">
          {['failed', 'timedOut', 'declined', 'cancelled'].includes(item.tool.status)
            ? <CircleAlert size={15} className="shrink-0 text-danger" />
            : item.tool.status === 'completed' ? <Check size={15} className="shrink-0 text-success" />
              : running ? <Loader2 size={15} className="shrink-0 animate-spin text-accent" />
                : <Terminal size={15} className="shrink-0 text-text-tertiary" />}
          <span className="min-w-0 flex-1 break-words font-mono text-xs">{item.tool.toolName}</span>
          <ChevronDown size={14} className="shrink-0 text-text-tertiary" />
        </summary>
        {item.tool.progressNote && <p className="mt-3 break-words text-xs leading-5 text-text-secondary">{item.tool.progressNote}</p>}
        {item.tool.arguments && <pre className="mt-3 max-h-40 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-surface-2 p-3 text-xs">{item.tool.arguments}</pre>}
        {item.tool.content && <div className="mt-3 max-h-80 overflow-auto"><StreamingMarkdown content={item.tool.content} isStreaming={false} reduceMotion /></div>}
      </details>
    ) : item.kind === 'thinking' ? (
      <details key={item.id} className="remote-thinking" data-timeline-kind="thinking">
        <summary className="cursor-pointer text-xs text-text-secondary">{t('remote.thinking')}</summary>
        <div className="mt-3 max-h-80 overflow-auto"><StreamingMarkdown content={item.text} isStreaming={running} reduceMotion /></div>
      </details>
    ) : (
      <article key={item.id} className="remote-answer" data-timeline-kind="answer">
        <p className="mb-3 text-xs font-semibold text-accent">Nexa</p>
        <StreamingMarkdown content={item.text} isStreaming={running} reduceMotion />
      </article>
    ))}
  </div>;
  return settled ? <details className="remote-activity">
    <summary className="cursor-pointer text-xs text-text-secondary">{t('remote.activity')}</summary>
    <div className="mt-4">{content}</div>
  </details> : content;
}
