import { memo, useLayoutEffect, useMemo, useRef } from 'react';
import type { Element, Root, RootContent } from 'hast';
import ReactMarkdown from 'react-markdown';
import { useStreamingPresentation } from '../../lib/useStreamingPresentation';
import { markdownProse } from '../../lib/markdownCodeProtection';

import { useTranslation } from '../../i18n';
import type { CitationCardData } from '../../lib/citationParser';
import { buildEvidenceItemsFromContent } from '../../lib/evidenceItems';
import { MAX_HIGHLIGHT_DOCUMENT_CHARS } from '../../lib/streaming/markdownPresentation';
import { CitationChip } from './EvidenceCard';
import {
  CitationContext,
  MarkdownRenderStateProvider,
  markdownComponents,
  markdownRemarkPlugins,
  preprocessChatContent,
  rehypePlugins,
} from './markdownComponents';

export interface MarkdownCitationLookup {
  getCard: (id: string) => CitationCardData | undefined;
}

const EMPTY_CITATION_LOOKUP: MarkdownCitationLookup = {
  getCard: () => undefined,
};

interface MarkdownDocumentProps {
  content: string;
  citationLookup: MarkdownCitationLookup;
  isStreaming: boolean;
  revealTail: boolean;
}

// Only the newest prose leaf needs animated runs. Leave code, math, links
// and earlier blocks in the normal Markdown renderer.
function rehypeStreamingTail() {
  return (root: Root) => {
    let parent: Root | Element = root;
    while (parent.children.length) {
      const child: RootContent = parent.children[parent.children.length - 1];
      if (child.type === 'text' && parent.type === 'element' && child.value.trim()) {
        parent.children[parent.children.length - 1] = {
          type: 'element', tagName: 'nexa-stream-tail', properties: {}, children: [child],
        };
        return;
      }
      if (child.type !== 'element' || !/^(p|li|ul|ol|blockquote|em|strong|del|h[1-6])$/.test(child.tagName)) return;
      parent = child;
    }
  };
}

function StreamingTail({ children = '' }: { children?: string }) {
  const previous = useRef({ text: '', starts: [] as number[] });
  const prior = previous.current;
  const appended = children.length - prior.text.length;
  let starts = prior.starts;
  if (children !== prior.text) {
    // A restored block or a large catch-up batch should be readable immediately.
    starts = appended > 0 && appended <= 160 && children.startsWith(prior.text)
      ? [...prior.starts.slice(-5), prior.text.length] : [];
  }
  useLayoutEffect(() => { previous.current = { text: children, starts }; }, [children, starts]);
  return <>{children.slice(0, starts[0] ?? children.length)}{starts.map((start, index) => (
    <span key={start} className="streaming-text-rise">{children.slice(start, starts[index + 1])}</span>
  ))}</>;
}

const livePlugins = [...rehypePlugins, rehypeStreamingTail];
const liveComponents: typeof markdownComponents = { ...markdownComponents, 'nexa-stream-tail': StreamingTail };

const MarkdownDocument = memo(function MarkdownDocument({
  content,
  citationLookup,
  isStreaming,
  revealTail,
}: MarkdownDocumentProps) {
  const { t } = useTranslation();
  const processed = useMemo(
    () => preprocessChatContent(content),
    [content],
  );
  const evidenceItems = useMemo(
    () => buildEvidenceItemsFromContent(
      markdownProse(content),
      citationLookup,
      (index) => t('chat.evidenceSourceLabel', { index: String(index) }),
    ),
    [citationLookup, content, t],
  );

  return (
    <>
      {evidenceItems.length > 0 && (
        <div className="mb-3 rounded-xl border border-border/70 bg-surface-1/70 px-2.5 py-2">
          <div className="mb-1 flex items-center justify-between gap-2">
            <span className="text-[11px] font-medium text-text-secondary">
              {t('chat.answerEvidence')}
            </span>
            <span className="text-[10px] text-text-tertiary">
              {t('chat.answerEvidenceSummary', { count: String(evidenceItems.length) })}
            </span>
          </div>
          <div className="flex flex-wrap gap-1.5">
            {evidenceItems.map((item) => (
              <CitationChip
                key={item.chunkId}
                chunkId={item.chunkId}
                displayText={item.displayText}
                card={item.card}
              />
            ))}
          </div>
        </div>
      )}
      <CitationContext.Provider value={citationLookup}>
        <MarkdownRenderStateProvider isStreaming={isStreaming} plainCode={content.length > MAX_HIGHLIGHT_DOCUMENT_CHARS}>
          <div
            className="prose-chat"
            data-testid="chat-markdown-document"
            data-markdown-source-chars={content.length}
          >
            <ReactMarkdown
              remarkPlugins={markdownRemarkPlugins}
              rehypePlugins={revealTail ? livePlugins : rehypePlugins}
              components={revealTail ? liveComponents : markdownComponents}
              urlTransform={(url) => url}
            >
              {processed}
            </ReactMarkdown>
          </div>
        </MarkdownRenderStateProvider>
      </CitationContext.Provider>
    </>
  );
});

export const StreamingMarkdown = memo(function StreamingMarkdown({
  content,
  isStreaming,
  partial = false,
  citationLookup,
  reduceMotion,
}: {
  content: string;
  isStreaming: boolean;
  partial?: boolean;
  citationLookup?: MarkdownCitationLookup;
  reduceMotion: boolean;
}) {
  const presented = useStreamingPresentation(content, isStreaming, reduceMotion);
  const effectiveCitationLookup = citationLookup ?? EMPTY_CITATION_LOOKUP;

  return (
    <div className="relative">
      <MarkdownDocument
        content={presented}
        citationLookup={effectiveCitationLookup}
        isStreaming={isStreaming || partial}
        revealTail={isStreaming && !reduceMotion}
      />
      {isStreaming && (
        <span className={`streaming-caret-overlay ${reduceMotion ? '' : 'animate-pulse'}`} />
      )}
    </div>
  );
});
