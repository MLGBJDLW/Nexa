import { useCallback, useMemo, useRef, useState, type ReactNode } from 'react';
import { Download, Loader2, X } from 'lucide-react';
import { FilePreviewContext } from '../preview/filePreviewContext';
import { StructuredPreviewRenderer, createPreviewLabels } from '../preview/StructuredPreview';
import { StreamingMarkdown } from '../../components/chat/StreamingMarkdown';
import type { FilePreview, getEvidenceCard } from '../../lib/api';
import { useTranslation } from '../../i18n';
import type { RemoteClient } from './remoteClient';
import { remoteButton } from './remoteUi';

export function RemotePreviewProvider({ client, children }: { client: RemoteClient; children: ReactNode }) {
  const { t } = useTranslation();
  const [path, setPath] = useState('');
  const [preview, setPreview] = useState<FilePreview | null>(null);
  const [media, setMedia] = useState('');
  const [htmlUrl, setHtmlUrl] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const generation = useRef(0);
  const labels = useMemo(() => createPreviewLabels(t), [t]);
  const resolveFileUrl = useCallback((path: string) => client.rpc<string>('files.data', { path }), [client]);
  const publishHtml = useCallback(async (html: string) => {
    const path = await client.rpc<string>('preview.html', { html });
    return new URL(path, client.state.endpoint!.url).href;
  }, [client]);
  const openCodePreview = useCallback((code: string, language: string) => {
    const mine = ++generation.current;
    setPath(language); setPreview(null); setMedia(''); setHtmlUrl(''); setError(''); setLoading(true);
    void publishHtml(code).then(url => { if (mine === generation.current) setHtmlUrl(url); })
      .catch(error => { if (mine === generation.current) setError(String(error)); }).finally(() => { if (mine === generation.current) setLoading(false); });
  }, [publishHtml]);
  const openWebLink = useCallback((url: string) => {
    try { if (/^https?:$/.test(new URL(url).protocol)) window.open(url, '_blank', 'noopener,noreferrer'); } catch { /* Unsupported protocols never execute. */ }
  }, []);
  const openFilePreview = useCallback((path: string) => {
    const mine = ++generation.current;
    setPath(path); setPreview(null); setMedia(''); setHtmlUrl(''); setError(''); setLoading(true);
    void client.rpc<FilePreview>('files.preview', { path }).then(async next => {
      if (mine !== generation.current) return;
      setPreview(next);
      if (['.html', '.htm'].includes(next.extension) && next.content != null) {
        const url = await publishHtml(next.content);
        if (mine === generation.current) setHtmlUrl(url);
      }
      if (['image', 'audio', 'video'].includes(next.kind) || next.extension === '.pdf') {
        const data = await resolveFileUrl(next.path);
        if (mine === generation.current) setMedia(data);
      }
    }).catch(error => { if (mine === generation.current) setError(error instanceof Error ? error.message : String(error)); })
      .finally(() => { if (mine === generation.current) setLoading(false); });
  }, [client, resolveFileUrl, publishHtml]);
  const context = useMemo(() => ({ openFilePreview, openWebLink, openCodePreview, resolveFileUrl, remote:true,
    loadEvidence:(chunkId: string) => client.rpc<Awaited<ReturnType<typeof getEvidenceCard>>>('evidence.get', { chunkId }),
  }), [client, openFilePreview, openWebLink, openCodePreview, resolveFileUrl]);
  async function download() {
    if (!preview) return;
    try {
      const data = media || await resolveFileUrl(preview.path);
      const blob = await (await fetch(data)).blob(); const url = URL.createObjectURL(blob);
      const link = document.createElement('a'); link.href = url; link.download = preview.displayName; link.click();
      setTimeout(() => URL.revokeObjectURL(url), 30_000);
    } catch (error) { setError(error instanceof Error ? error.message : String(error)); }
  }
  return <FilePreviewContext.Provider value={context}>{children}
    {path && <div className="fixed inset-0 z-50 flex flex-col bg-surface-0" role="dialog" aria-modal="true" aria-label={t('preview.title')}>
      <header className="flex shrink-0 items-center gap-2 border-b border-border p-3"><strong className="min-w-0 flex-1 truncate text-sm">{preview?.displayName || path}</strong>
        <button className={remoteButton} onClick={() => void download()} disabled={!preview} aria-label={t('remote.download')}><Download size={16} /></button>
        <button className={remoteButton} autoFocus onClick={() => { generation.current++; setPath(''); }} aria-label={t('common.close')}><X size={18} /></button>
      </header>
      <div className="min-h-0 flex-1 overflow-auto p-4">
        {loading && <Loader2 className="animate-spin" />}{error && <p role="alert" className="mb-4 text-sm text-danger">{error}</p>}
        {preview?.warning && <p className="mb-3 text-xs text-text-secondary">{preview.warning}</p>}
        {htmlUrl ? <iframe src={htmlUrl} sandbox="allow-scripts" referrerPolicy="no-referrer" title={preview?.displayName || path} className="h-full min-h-[70vh] w-full border-0 bg-white" /> : preview && (preview.kind === 'image' && media ? <img src={media} alt={preview.displayName} className="mx-auto max-h-full max-w-full object-contain" />
          : preview.kind === 'audio' && media ? <audio src={media} controls className="w-full" />
          : preview.kind === 'video' && media ? <video src={media} controls playsInline className="max-w-full" />
          : preview.extension === '.pdf' && media ? <iframe src={media} sandbox="" title={preview.displayName} className="h-full min-h-[70vh] w-full border-0" />
          : preview.structuredPreview ? <StructuredPreviewRenderer preview={preview.structuredPreview} labels={labels} onMouseUp={() => {}} onOpenWebLink={openWebLink} />
          : preview.content != null ? <StreamingMarkdown content={preview.extension === '.md' || preview.extension === '.markdown' ? preview.content : `\n\`\`\`\`${preview.language || ''}\n${preview.content}\n\`\`\`\``} isStreaming={false} reduceMotion /> : <p>{t('preview.unsupported')}</p>)}
      </div>
    </div>}
  </FilePreviewContext.Provider>;
}
