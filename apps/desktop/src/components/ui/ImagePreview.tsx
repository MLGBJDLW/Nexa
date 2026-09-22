import { useCallback, useEffect, useRef, useState, type ComponentPropsWithoutRef } from 'react';
import { createPortal } from 'react-dom';
import { X, ZoomIn, ZoomOut, RotateCcw } from 'lucide-react';
import { useTranslation } from '../../i18n';

/** Use the already resolved image URL, including remote-session asset URLs.
 * A native modal lives above transformed/scrolling chat containers and supplies
 * keyboard focus containment without making the full-size image a new page. */
export function ImagePreview({ src, alt = '', className, ...props }: ComponentPropsWithoutRef<'img'>) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [zoom, setZoom] = useState(1);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [viewport, setViewport] = useState({ width: window.innerWidth, height: window.innerHeight });
  const dialogRef = useRef<HTMLDialogElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const close = useCallback(() => {
    if (dialogRef.current?.open) dialogRef.current.close();
    setOpen(false);
    triggerRef.current?.focus();
  }, []);
  useEffect(() => {
    setOpen(false);
    setZoom(1);
    setSize({ width: 0, height: 0 });
  }, [src]);
  useEffect(() => {
    if (!open) return;
    dialogRef.current?.showModal();
    const resize = () => setViewport({ width: window.innerWidth, height: window.innerHeight });
    resize();
    window.addEventListener('resize', resize);
    return () => window.removeEventListener('resize', resize);
  }, [open]);
  const fittedWidth = size.width ? Math.min(size.width, Math.max(1, viewport.width * 0.94 - 28), Math.max(1, viewport.height * 0.9 - 84) * size.width / size.height) : undefined;
  return <>
    <button
      type="button"
      ref={triggerRef}
      className="inline-flex max-w-full cursor-zoom-in rounded-md align-middle focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
      aria-label={`${t('preview.preview')}: ${alt || t('chat.generatedImageAlt')}`}
      onClick={event => { event.preventDefault(); event.stopPropagation(); setZoom(1); setOpen(true); }}
    >
      <img {...props} src={src} alt={alt} className={className} />
    </button>
    {open && createPortal(
      <dialog
        ref={dialogRef}
        data-testid="image-lightbox"
        aria-label={alt || t('chat.generatedImageAlt')}
        className="fixed inset-0 m-auto h-[90dvh] w-[94vw] max-w-none overflow-hidden rounded-xl border border-border bg-surface-1 p-0 text-text-primary shadow-2xl backdrop:bg-black/70"
        onCancel={event => { event.preventDefault(); close(); }}
        onClose={close}
        onClick={event => { if (event.target === event.currentTarget) close(); }}
      >
        <div className="flex h-full min-h-0 flex-col">
          <div className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2">
            <span className="min-w-0 flex-1 truncate text-sm" title={alt}>{alt}</span>
            <button type="button" className="rounded p-2 hover:bg-surface-3 disabled:opacity-40" aria-label={t('preview.zoomOut')} disabled={zoom <= 0.25} onClick={() => setZoom(value => Math.max(0.25, value / 1.5))}><ZoomOut size={18} /></button>
            <button type="button" className="rounded px-2 py-1 text-xs hover:bg-surface-3" aria-label={t('preview.resetZoom')} onClick={() => setZoom(1)}><RotateCcw size={16} /></button>
            <button type="button" className="rounded p-2 hover:bg-surface-3 disabled:opacity-40" aria-label={t('preview.zoomIn')} disabled={zoom >= 8} onClick={() => setZoom(value => Math.min(8, value * 1.5))}><ZoomIn size={18} /></button>
            <button type="button" autoFocus className="rounded p-2 hover:bg-surface-3" aria-label={t('common.close')} onClick={close}><X size={18} /></button>
          </div>
          <div className="min-h-0 flex-1 overflow-auto p-3">
            <div className="flex min-h-full min-w-full items-center justify-center" style={{ width: fittedWidth ? fittedWidth * zoom : undefined }}>
              <img src={src} alt={alt} className="block max-w-none object-contain" style={{ width: fittedWidth ? fittedWidth * zoom : undefined, maxHeight: fittedWidth ? undefined : '72vh' }} onLoad={event => setSize({ width: event.currentTarget.naturalWidth, height: event.currentTarget.naturalHeight })} />
            </div>
          </div>
        </div>
      </dialog>, document.body,
    )}
  </>;
}
