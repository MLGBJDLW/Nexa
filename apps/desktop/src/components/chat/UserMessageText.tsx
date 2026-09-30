import { useEffect, useId, useRef, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { useTranslation } from '../../i18n';
import { pauseScrollFollowForReading } from '../../lib/scrollFollow';

/** No executable diagrams, HTML, or automatic remote image requests in user drafts. */
export function UserMarkdown({ text }: { text: string }) {
  return <div className="user-markdown prose-chat min-w-0 [overflow-wrap:anywhere]">
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={{
      a: ({ children, href }) => <a href={href} target="_blank" rel="noopener noreferrer">{children}</a>,
      img: ({ alt }) => <span>{alt}</span>,
    }}>{text}</ReactMarkdown>
  </div>;
}

export function UserMessageText({ text }: { text: string }) {
  const { t } = useTranslation();
  const id = useId();
  const contentRef = useRef<HTMLDivElement>(null);
  const [expanded, setExpanded] = useState(false);
  const [overflowing, setOverflowing] = useState(false);
  const [source, setSource] = useState(false);
  // Measure rendered height, including wrapped lines and tables, at the current font size.
  useEffect(() => {
    const content = contentRef.current;
    if (!content) return;
    const measure = () => setOverflowing(content.getBoundingClientRect().height > 120 + 1);
    const observer = new ResizeObserver(measure);
    observer.observe(content);
    measure();
    return () => observer.disconnect();
  }, [text, source]);
  const collapsed = overflowing && !expanded;
  const hasMarkdown = /[\x60*_[\]#>|~]|^\s*[-+]\s/m.test(text);
  return <div className="min-w-0" data-testid="chat-user-message-body">
    <div id={id} className="relative overflow-hidden" style={collapsed ? { maxHeight: 120 } : undefined}>
      <div ref={contentRef} data-testid="chat-user-message-text" className="[overflow-wrap:anywhere]" inert={collapsed || undefined}>
        {source ? <div className="whitespace-pre-wrap [overflow-wrap:anywhere]">{text}</div> : <UserMarkdown text={text} />}
      </div>
      {collapsed && <div aria-hidden="true" className="pointer-events-none absolute inset-x-0 bottom-0 h-5 bg-linear-to-t from-surface-1/80 to-transparent" />}
    </div>
    {(overflowing || hasMarkdown) && <div className="mt-1 flex items-center gap-3 text-[11px] text-text-secondary">
      {overflowing && <button type="button" aria-expanded={expanded} aria-controls={id} onClick={() => { pauseScrollFollowForReading(contentRef.current); setExpanded(!expanded); }} className="rounded py-1 hover:text-accent focus-visible:outline-accent">{t(expanded ? 'chat.collapseMessage' : 'chat.expandMessage')}</button>}
      <button type="button" aria-pressed={source} onClick={() => setSource(!source)} className="rounded py-1 opacity-65 hover:text-accent hover:opacity-100 focus-visible:outline-accent">{t(source ? 'chat.renderMarkdown' : 'chat.viewSource')}</button>
    </div>}
  </div>;
}
