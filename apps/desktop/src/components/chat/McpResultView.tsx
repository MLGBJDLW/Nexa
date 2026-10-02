import { useTranslation } from '../../i18n';
import { mcpExternalLink, mcpInlineMedia, type McpContentBlock, type McpToolResult } from '../../lib/mcpResult';
import { ImagePreview } from '../ui/ImagePreview';

function ResourceLink({ uri, label }: { uri: string; label: string }) {
  const href = mcpExternalLink(uri);
  return href ? <a href={href} target="_blank" rel="noopener noreferrer" className="break-all text-accent underline underline-offset-2">{label}</a> : <span className="break-all text-text-secondary" title={uri}>{label}</span>;
}

function ContentBlock({ block }: { block: McpContentBlock }) {
  const { t } = useTranslation();
  if (block.type === 'text') return <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words font-sans text-xs">{block.text}</pre>;
  if (block.type === 'image' || block.type === 'audio') {
    const src = mcpInlineMedia(block.data,block.mimeType,block.type);
    if (!src) return <span className="text-xs text-text-tertiary">{t('chat.mcpMediaUnavailable')}</span>;
    return block.type === 'image'
      ? <ImagePreview src={src} alt={t('chat.mcpImage')} className="min-h-10 min-w-10 max-h-72 max-w-full rounded-md object-contain" loading="lazy" />
      : <audio src={src} controls preload="none" aria-label={t('chat.mcpAudio')} className="h-10 max-w-full" />;
  }
  if (block.type === 'resource_link') return <ResourceLink uri={block.uri} label={block.title || block.name || block.uri} />;
  if (block.type === 'resource') return <div className="space-y-1">
    <ResourceLink uri={block.resource.uri} label={block.resource.uri} />
    {typeof block.resource.text === 'string' && <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words text-xs">{block.resource.text}</pre>}
    {block.resource.blob && <span className="text-xs text-text-tertiary">{t('chat.mcpEmbeddedResource')} · {block.resource.mimeType || 'application/octet-stream'}</span>}
  </div>;
  if (block.type === 'unsupported') return <p className="text-xs text-text-tertiary">{block.message}</p>;
  return null;
}

export function McpResultView({ result }: { result: McpToolResult }) {
  const { t } = useTranslation();
  return <div className="space-y-3 text-xs" data-testid="mcp-result">
    {result.toolIdentity && <p className="text-text-tertiary">{result.toolIdentity.connectorId}</p>}
    {result.contentBlocks.map((block,index) => <div key={index} className="min-w-0"><ContentBlock block={block} /></div>)}
    {result.structuredContent != null && <details className="rounded-md border border-border/40 p-2" data-testid="mcp-structured-content">
      <summary className="cursor-pointer text-text-secondary">{t('chat.mcpStructuredContent')}</summary>
      <pre className="mt-2 max-h-72 overflow-auto whitespace-pre-wrap break-words">{JSON.stringify(result.structuredContent,null,2)}</pre>
    </details>}
    {result.notices.map((notice,index) => <p key={index} className="text-text-tertiary">{notice}</p>)}
  </div>;
}
