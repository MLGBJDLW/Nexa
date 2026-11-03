import { useEffect, useRef, useState, type PointerEvent } from 'react';
import { createPortal } from 'react-dom';
import { Brush, Eraser, Undo2, RotateCcw, X, ZoomIn, ZoomOut } from 'lucide-react';
import { useTranslation } from '../../i18n';
import type { ImageAttachment } from '../../types/conversation';
import type { ImageEditSubmission } from './ImageEditContext';

type Point = { x: number; y: number };
type Stroke = { erase: boolean; radius: number; points: Point[] };
const MAX_BYTES = 12 * 1024 * 1024;
const MAX_PIXELS = 16_000_000;

function paint(context: CanvasRenderingContext2D, stroke: Stroke, width: number, height: number, color: string) {
  context.globalCompositeOperation = stroke.erase ? 'destination-out' : 'source-over';
  context.strokeStyle = color; context.fillStyle = color;
  context.lineCap = 'round'; context.lineJoin = 'round';
  context.lineWidth = stroke.radius * width * 2;
  const first = stroke.points[0];
  if (!first) return;
  context.beginPath(); context.arc(first.x * width, first.y * height, stroke.radius * width, 0, Math.PI * 2); context.fill();
  context.beginPath(); context.moveTo(first.x * width, first.y * height);
  for (const point of stroke.points.slice(1)) context.lineTo(point.x * width, point.y * height);
  context.stroke(); context.globalCompositeOperation = 'source-over';
}
function canvasBlob(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => canvas.toBlob(blob => blob ? resolve(blob) : reject(new Error('Image encoding failed')), 'image/png'));
}
async function attachment(canvas: HTMLCanvasElement, name: string): Promise<ImageAttachment> {
  const blob = await canvasBlob(canvas);
  if (blob.size > MAX_BYTES) throw new Error('imageSize');
  return new Promise((resolve, reject) => {
    const reader = new FileReader(); reader.onerror = reject;
    reader.onload = () => resolve({ base64Data: String(reader.result).split(',')[1], mediaType: 'image/png', originalName: name });
    reader.readAsDataURL(blob);
  });
}

export default function ImageEditorDialog({ image, onClose, onSubmit }: {
  image: { src: string; name: string }; onClose: () => void; onSubmit: (value: ImageEditSubmission) => Promise<void>;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDialogElement>(null);
  const overlay = useRef<HTMLCanvasElement>(null);
  const bitmap = useRef<ImageBitmap | null>(null);
  const strokes = useRef<Stroke[]>([]);
  const drawing = useRef<{ id: number; stroke: Stroke } | null>(null);
  const alive = useRef(true);
  const [size, setSize] = useState<{ width: number; height: number } | null>(null);
  const [revision, setRevision] = useState(0);
  const [tool, setTool] = useState<'brush' | 'eraser'>('brush');
  const [brush, setBrush] = useState(36);
  const [zoom, setZoom] = useState(1);
  const [compare, setCompare] = useState(false);
  const [prompt, setPrompt] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [readyUrl, setReadyUrl] = useState('');
  const previewWidth = size ? Math.min(1280, size.width) : 1;
  const previewHeight = size ? Math.round(previewWidth * size.height / size.width) : 1;
  useEffect(() => {
    alive.current = true;
    dialog.current?.showModal();
    const controller = new AbortController();
    let objectUrl = '';
    void (async () => {
      const response = await fetch(image.src, { signal: controller.signal });
      if (!response.ok) throw new Error('imageLoad');
      if (Number(response.headers.get('content-length')) > MAX_BYTES) throw new Error('imageSize');
      const reader = response.body?.getReader();
      if (!reader) throw new Error('imageLoad');
      const chunks: Uint8Array<ArrayBuffer>[] = []; let bytes = 0;
      for (;;) {
        const part = await reader.read(); if (part.done) break;
        bytes += part.value.byteLength;
        if (bytes > MAX_BYTES) { await reader.cancel(); throw new Error('imageSize'); }
        chunks.push(part.value);
      }
      const blob = new Blob(chunks);
      const decoded = await createImageBitmap(blob);
      if (!alive.current) { decoded.close(); return; }
      if (decoded.width > 8192 || decoded.height > 8192 || decoded.width * decoded.height > MAX_PIXELS) { decoded.close(); throw new Error('imageSize'); }
      bitmap.current = decoded;
      objectUrl = URL.createObjectURL(blob); setReadyUrl(objectUrl);
      setSize({ width: decoded.width, height: decoded.height });
    })().catch(reason => {
      if (alive.current && !controller.signal.aborted) setError(t(reason?.message === 'imageSize' ? 'chat.imageEditSizeError' : 'chat.imageEditLoadError'));
    });
    return () => { alive.current = false; controller.abort(); bitmap.current?.close(); bitmap.current = null; if (objectUrl) URL.revokeObjectURL(objectUrl); };
  }, [image.src, t]);
  const redraw = () => {
    const canvas = overlay.current; const context = canvas?.getContext('2d'); if (!canvas || !context) return;
    context.clearRect(0, 0, canvas.width, canvas.height);
    for (const stroke of strokes.current) paint(context, stroke, canvas.width, canvas.height, '#e879f9');
  };
  const point = (event: PointerEvent<HTMLCanvasElement>): Point => {
    const rect = event.currentTarget.getBoundingClientRect();
    return { x: Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width)), y: Math.max(0, Math.min(1, (event.clientY - rect.top) / rect.height)) };
  };
  const start = (event: PointerEvent<HTMLCanvasElement>) => {
    if (busy || compare || !size || !event.isPrimary || event.button !== 0) return;
    event.preventDefault(); event.currentTarget.setPointerCapture(event.pointerId);
    const stroke: Stroke = { erase: tool === 'eraser', radius: brush / 2 / event.currentTarget.getBoundingClientRect().width, points: [point(event)] };
    strokes.current.push(stroke); drawing.current = { id: event.pointerId, stroke };
    redraw();
  };
  const move = (event: PointerEvent<HTMLCanvasElement>) => {
    const active = drawing.current; if (!active || active.id !== event.pointerId) return;
    const next = point(event); const previous = active.stroke.points[active.stroke.points.length - 1];
    if (Math.hypot((next.x - previous.x) * previewWidth, (next.y - previous.y) * previewHeight) < 0.8) return;
    active.stroke.points.push(next);
    const context = overlay.current?.getContext('2d');
    if (context) paint(context, { ...active.stroke, points: [previous, next] }, previewWidth, previewHeight, '#e879f9');
  };
  const finish = (event: PointerEvent<HTMLCanvasElement>) => {
    if (drawing.current?.id !== event.pointerId) return;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    drawing.current = null; setRevision(value => value + 1);
  };
  const submit = async () => {
    if (!bitmap.current || !size || busy || !prompt.trim()) return;
    const selection = overlay.current?.getContext('2d')?.getImageData(0, 0, previewWidth, previewHeight).data;
    if (!selection?.some((value, index) => index % 4 === 3 && value > 0)) { setError(t('chat.imageEditSelectRequired')); return; }
    setBusy(true); setError('');
    const canvas = document.createElement('canvas'); canvas.width = size.width; canvas.height = size.height;
    const context = canvas.getContext('2d')!;
    try {
      context.drawImage(bitmap.current, 0, 0);
      const original = await attachment(canvas, 'edit-original.png');
      context.clearRect(0, 0, size.width, size.height);
      for (const stroke of strokes.current) paint(context, stroke, size.width, size.height, '#ffffff');
      // An opaque background with transparent selected pixels is the mask API contract.
      context.globalCompositeOperation = 'source-out'; context.fillStyle = '#ffffff'; context.fillRect(0, 0, size.width, size.height);
      context.globalCompositeOperation = 'source-over';
      const mask = await attachment(canvas, 'edit-mask.png');
      context.clearRect(0, 0, size.width, size.height);
      for (const stroke of strokes.current) paint(context, stroke, size.width, size.height, '#e879f9');
      context.globalCompositeOperation = 'destination-over'; context.drawImage(bitmap.current!, 0, 0);
      context.globalCompositeOperation = 'source-over';
      const guide = await attachment(canvas, 'edit-guide.png');
      if ([original, mask, guide].reduce((bytes, image) => bytes + image.base64Data.length * 3 / 4, 0) > 32 * 1024 * 1024) throw new Error('imageSize');
      if (!alive.current) return;
      await onSubmit({ prompt: prompt.trim(), attachments: [original, mask, guide], artifacts: { kind: 'imageEdit', version: 1, original: original.originalName, mask: mask.originalName, guide: guide.originalName } });
      if (alive.current) onClose();
    } catch (reason) {
      if (alive.current) setError(reason instanceof Error && reason.message === 'imageSize' ? t('chat.imageEditSizeError') : t('chat.imageEditSubmitError'));
    } finally { canvas.width = 0; canvas.height = 0; if (alive.current) setBusy(false); }
  };
  return createPortal(<dialog ref={dialog} data-testid="image-editor" aria-label={t('chat.imageEditTitle')} onCancel={event => { event.preventDefault(); if (!busy) onClose(); }} className="fixed inset-0 m-auto h-[min(92dvh,900px)] w-[min(96vw,1200px)] max-w-none overflow-hidden rounded-2xl border border-border bg-surface-1 p-0 text-text-primary shadow-2xl backdrop:bg-black/70">
    <div className="flex h-full flex-col">
      <header className="flex items-center gap-3 border-b border-border px-4 py-3"><div className="min-w-0 flex-1"><h2 className="text-sm font-semibold">{t('chat.imageEditTitle')}</h2><p className="mt-0.5 truncate text-xs text-text-tertiary">{image.name}</p></div><button type="button" disabled={busy} onClick={onClose} aria-label={t('common.close')} className="rounded p-2 hover:bg-surface-2"><X size={18} /></button></header>
      <div role="toolbar" aria-label={t('chat.imageEditTools')} className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border px-4 py-2">
        {(['brush', 'eraser'] as const).map(value => <button key={value} type="button" disabled={busy} aria-pressed={tool === value} aria-label={t(value === 'brush' ? 'chat.imageEditBrush' : 'chat.imageEditEraser')} onClick={() => { setTool(value); setCompare(false); }} className={`rounded-lg p-2 ${tool === value ? 'bg-accent/15 text-accent' : 'hover:bg-surface-2'}`}>{value === 'brush' ? <Brush size={17} /> : <Eraser size={17} />}</button>)}
        <label className="flex items-center gap-2 text-xs text-text-secondary">{t('chat.imageEditBrushSize')}<input aria-label={t('chat.imageEditBrushSize')} disabled={busy} type="range" min="4" max="160" value={brush} onChange={event => setBrush(Number(event.target.value))} className="w-24 accent-accent" /></label>
        <button type="button" disabled={busy || revision === 0 || strokes.current.length === 0} aria-label={t('chat.imageEditUndo')} onClick={() => { strokes.current.pop(); redraw(); setRevision(value => value + 1); }} className="rounded p-2 hover:bg-surface-2 disabled:opacity-40"><Undo2 size={17} /></button>
        <button type="button" disabled={busy || !strokes.current.length} aria-label={t('chat.imageEditClear')} onClick={() => { strokes.current = []; redraw(); setRevision(value => value + 1); }} className="rounded p-2 hover:bg-surface-2 disabled:opacity-40"><RotateCcw size={17} /></button>
        <div className="ml-auto flex items-center gap-2"><button type="button" aria-label={t('preview.zoomOut')} disabled={zoom <= 1} onClick={() => setZoom(value => Math.max(1, value / 1.5))} className="rounded p-2 disabled:opacity-40"><ZoomOut size={17} /></button><span className="text-xs tabular-nums">{Math.round(zoom * 100)}%</span><button type="button" aria-label={t('preview.zoomIn')} disabled={zoom >= 4} onClick={() => setZoom(value => Math.min(4, value * 1.5))} className="rounded p-2 disabled:opacity-40"><ZoomIn size={17} /></button><label className="flex items-center gap-1.5 text-xs"><input type="checkbox" checked={compare} onChange={event => setCompare(event.target.checked)} />{t('chat.imageEditOriginal')}</label></div>
      </div>
      <div className="min-h-0 flex-1 overflow-auto bg-surface-0 p-4">
        {readyUrl && <div className="relative mx-auto" style={{ width: `${zoom * 100}%`, maxWidth: zoom === 1 ? `min(100%, ${previewWidth}px)` : undefined }}><img src={readyUrl} alt={image.name} draggable={false} className="block w-full select-none" /><canvas ref={overlay} width={previewWidth} height={previewHeight} data-testid="image-edit-canvas" aria-label={t('chat.imageEditCanvas')} className="absolute inset-0 h-full w-full touch-none cursor-crosshair" style={{ opacity: compare ? 0 : 0.5 }} onPointerDown={start} onPointerMove={move} onPointerUp={finish} onPointerCancel={finish} /></div>}
        {!readyUrl && !error && <p role="status" className="p-6 text-center text-sm text-text-tertiary">{t('common.loading')}</p>}
      </div>
      <footer className="shrink-0 border-t border-border p-4"><p className="mb-2 text-xs text-text-tertiary">{t('chat.imageEditHint')}</p><label className="sr-only" htmlFor="image-edit-prompt">{t('chat.imageEditPrompt')}</label><textarea id="image-edit-prompt" autoFocus value={prompt} disabled={busy} onChange={event => setPrompt(event.target.value)} placeholder={t('chat.imageEditPrompt')} rows={2} className="w-full resize-none rounded-lg border border-border bg-surface-0 px-3 py-2 text-sm outline-none focus:border-accent" />{error && <p role="alert" className="mt-2 text-xs text-danger">{error}</p>}<div className="mt-2 flex justify-end gap-2"><button type="button" disabled={busy} onClick={onClose} className="rounded-lg px-3 py-2 text-sm hover:bg-surface-2">{t('common.cancel')}</button><button type="button" disabled={busy || !size || !prompt.trim() || !strokes.current.length} onClick={() => void submit()} className="rounded-lg bg-accent px-4 py-2 text-sm text-white disabled:opacity-40">{t(busy ? 'chat.imageEditSending' : 'chat.imageEditSubmit')}</button></div></footer>
    </div>
  </dialog>, document.body);
}
