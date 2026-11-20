import { memo, useDeferredValue, useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { Root } from 'mdast';
import type { VFile } from 'vfile';
import { useTranslation } from '../../i18n';
import { pauseScrollFollowForReading } from '../../lib/scrollFollow';

function omitUnformattedDraft() {
  return (tree: Root, file: VFile) => {
    const [block] = tree.children;
    if (tree.children.length === 1 && block.type === 'paragraph' && block.children.length === 1
      && block.children[0].type === 'text' && block.children[0].value === String(file).trim()) {
      tree.children = [];
    }
  };
}
const messagePlugins = [remarkGfm];
const draftPlugins = [remarkGfm, omitUnformattedDraft];

/** No executable diagrams, HTML, or automatic remote image requests in user drafts. */
export const UserMarkdown = memo(function UserMarkdown({ text, drafting = false }: { text: string; drafting?: boolean }) {
  return <div className="user-markdown prose-chat min-w-0 [overflow-wrap:anywhere]">
    <ReactMarkdown remarkPlugins={drafting ? draftPlugins : messagePlugins} components={{
      a: ({ children, href }) => drafting ? <span className="text-accent underline">{children}</span> : <a href={href} target="_blank" rel="noopener noreferrer">{children}</a>,
      img: ({ alt }) => <span>{alt}</span>,
    }}>{text}</ReactMarkdown>
  </div>;
});

function LiveDraftMarkdown({ text }: { text: string }) {
  const rendered = useDeferredValue(text);
  // Keep simple typing cheap. Let the same GFM parser used by sent messages
  // decide whether punctuation, URLs, Setext headings or code change the display.
  if (!text.trim() || !rendered.trim() || /^[\p{L}\p{N} ]+$/u.test(rendered)) return null;
  return <div data-testid="chat-draft-markdown" className="live-draft-markdown max-h-40 overflow-auto px-4 py-2 text-sm text-text-primary">
    <UserMarkdown text={rendered} drafting />
  </div>;
}

export function DraftEditRegion({ draftKey, text, sourceClassName, children }: { draftKey: string; text: string; sourceClassName: string; children: ReactNode }) {
  const region = useRef<HTMLDivElement>(null);
  const [editing, setEditing] = useState(false);
  useLayoutEffect(() => {
    const editor = region.current;
    if (!editing || !editor) return;
    const doc = editor.ownerDocument;
    let pointerDown = false;
    let frame = 0;
    // A blur occurs on pointerdown. Collapsing the source then can move the
    // toolbar or message under that pointer before its click is dispatched.
    const settle = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (!pointerDown && !editor.contains(doc.activeElement)) setEditing(false);
      });
    };
    const press = () => { pointerDown = true; };
    const release = () => { pointerDown = false; settle(); };
    const leave = () => { if (!pointerDown) settle(); };
    doc.addEventListener('pointerdown', press, true);
    doc.addEventListener('pointerup', release, true);
    doc.addEventListener('pointercancel', release, true);
    editor.addEventListener('focusout', leave);
    return () => {
      cancelAnimationFrame(frame);
      doc.removeEventListener('pointerdown', press, true);
      doc.removeEventListener('pointerup', release, true);
      doc.removeEventListener('pointercancel', release, true);
      editor.removeEventListener('focusout', leave);
    };
  }, [editing]);
  return <div ref={region} className="draft-edit-region" data-editing={editing} onFocusCapture={() => setEditing(true)}>
    <div className={`draft-source ${sourceClassName}`}>{children}</div>
    <LiveDraftMarkdown key={draftKey} text={text} />
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
