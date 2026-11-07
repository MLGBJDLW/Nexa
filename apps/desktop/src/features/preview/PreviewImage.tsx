import { useEffect, useState, type ComponentPropsWithoutRef } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { useFilePreview } from './filePreviewContext';
import { localFileReference, isInlineFileReference } from './localFileReference';
import { ImagePreview } from '../../components/ui/ImagePreview';

export function PreviewImage({ src, node: _node, ...props }: ComponentPropsWithoutRef<'img'> & { node?: unknown }) {
  const { resolveFileUrl, openFilePreview, openWebLink } = useFilePreview();
  const [url, setUrl] = useState('');
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let active = true;
    setFailed(false); setUrl('');
    if (!src) return;
    if (/^(https?:|data:image\/(png|jpeg|gif|webp);base64,|blob:)/i.test(src)) setUrl(src);
    else {
      const path = localFileReference(src);
      if (!isInlineFileReference(path)) { setFailed(true); return; }
      if (resolveFileUrl) void resolveFileUrl(path).then(url => { if (active) setUrl(url); }).catch(() => { if (active) setFailed(true); });
      else { try { setUrl(convertFileSrc(path)); } catch { setFailed(true); } }
    }
    return () => { active = false; };
  }, [src, resolveFileUrl]);
  if (failed) return <button type="button" className="text-sm text-accent underline" onClick={() => {
    if (src) /^https?:/i.test(src) ? openWebLink(src) : openFilePreview(localFileReference(src));
  }}>{props.alt || src}</button>;
  return url ? <ImagePreview {...props} src={url} loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setFailed(true)} /> : <span className="text-xs text-text-tertiary">{props.alt}</span>;
}
