import { useEffect, useRef, useState } from 'react';
import type { ImageAttachment } from '../../types/conversation';
import { ImagePreview } from '../ui/ImagePreview';

export interface MessageAttachmentSummary { index: number; mediaType: string; originalName: string }
export type LoadMessageAttachment = (messageId: string, index: number) => Promise<ImageAttachment>;

/** History carries metadata; only visible images request their persisted bytes. */
export function MessageImage({ messageId, attachment, load }: {
  messageId: string; attachment: MessageAttachmentSummary; load: LoadMessageAttachment;
}) {
  const host = useRef<HTMLSpanElement>(null);
  const [url, setUrl] = useState('');
  const [error, setError] = useState('');
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let active = true;
    setUrl(''); setError('');
    const observer = new IntersectionObserver(entries => {
      if (!entries.some(entry => entry.isIntersecting)) return;
      observer.disconnect();
      void load(messageId, attachment.index).then(image => {
        if (!active) return;
        if (!/^image\/(png|jpeg|webp|gif)$/.test(image.mediaType) || !image.base64Data) throw new Error('Image unavailable');
        setUrl(`data:${image.mediaType};base64,${image.base64Data}`);
      }).catch(cause => { if (active) setError(String(cause)); });
    }, { rootMargin: '200px' });
    if (host.current) observer.observe(host.current);
    return () => { active = false; observer.disconnect(); };
  }, [messageId, attachment.index, load, attempt]);
  return <span ref={host} className="inline-flex min-h-12 min-w-12 max-w-full items-center rounded-md border border-border bg-surface-1" data-message-attachment>
    {url ? <ImagePreview src={url} alt={attachment.originalName} className="max-h-[200px] max-w-full object-contain" />
      : error ? <button type="button" className="p-2 text-xs text-accent underline" title={error} onClick={() => setAttempt(value => value + 1)}>{attachment.originalName}</button>
      : <span className="p-2 text-xs text-text-tertiary" role="status">{attachment.originalName}</span>}
  </span>;
}
