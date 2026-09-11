import { useEffect, useState, type ComponentPropsWithoutRef } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { useFilePreview } from './filePreviewContext';
import { localFileReference } from './localFileReference';

export function PreviewImage({ src, node: _node, ...props }: ComponentPropsWithoutRef<'img'> & { node?: unknown }) {
  const { resolveFileUrl, openFilePreview } = useFilePreview();
  const [url, setUrl] = useState('');
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let active = true;
    setFailed(false); setUrl('');
    if (!src) return;
    if (/^(https?:|data:image\/(png|jpeg|gif|webp);base64,|blob:)/i.test(src)) setUrl(src);
    else {
      const path = localFileReference(src);
      if (/^[a-z][a-z0-9+.-]*:/i.test(path) && !/^[a-z]:[\\/]/i.test(path)) { setFailed(true); return; }
      if (resolveFileUrl) void resolveFileUrl(path).then(url => { if (active) setUrl(url); }).catch(() => { if (active) setFailed(true); });
      else { try { setUrl(convertFileSrc(path)); } catch { setFailed(true); } }
    }
    return () => { active = false; };
  }, [src, resolveFileUrl]);
  if (failed) return <button type="button" className="text-sm text-accent underline" onClick={() => src && openFilePreview(localFileReference(src))}>{props.alt || src}</button>;
  return url ? <img {...props} src={url} loading="lazy" onError={() => setFailed(true)} /> : <span className="text-xs text-text-tertiary">{props.alt}</span>;
}
