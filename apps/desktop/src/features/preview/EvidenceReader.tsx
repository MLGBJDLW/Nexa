import { useCallback, useEffect, useRef, useState } from 'react';
import { Loader2, ExternalLink } from 'lucide-react';
import { Modal } from '../../components/ui/Modal';
import { Button } from '../../components/ui/Button';
import { Badge } from '../../components/ui/Badge';
import { useTranslation } from '../../i18n';
import * as api from '../../lib/api';
import { formatUserError } from '../../lib/userError';
import { isWebUrl } from '../../lib/sourceDisplay';
import type { EvidenceContext, EvidenceLocator, EvidenceRef, DocumentOutline } from '../../types/evidence';
import { useFilePreview } from './filePreviewContext';

type Translate = ReturnType<typeof useTranslation>['t'];
export function evidenceLocationLabel(locator: EvidenceLocator, t: Translate): string {
  switch (locator.kind) {
    case 'text': return `${t('preview.lines')} ${locator.lineStart}–${locator.lineEnd}`;
    case 'pdf': return `${t('preview.page')} ${locator.page}`;
    case 'document': return locator.table ? t('citation.tableRow', { table: locator.table, row: locator.row ?? 1 }) : t('citation.paragraph', { number: locator.paragraph });
    case 'sheet': return `${locator.sheet}!${locator.range}`;
    case 'slide': return t('citation.slide', { number: locator.slide });
    case 'media': return `${(locator.startMs / 1000).toFixed(1)}–${(locator.endMs / 1000).toFixed(1)} s`;
    case 'extracted': return locator.section;
    default: return t('citation.locationUnknown');
  }
}

export function EvidenceReader({ reference, onClose }: { reference: EvidenceRef; onClose: () => void }) {
  const { t } = useTranslation();
  const { openFilePreview, openWebLink, loadEvidenceContext = api.getEvidenceContext, loadDocumentOutline = api.getDocumentOutline } = useFilePreview();
  const [selected, setSelected] = useState(reference);
  const [loaded, setData] = useState<EvidenceContext | null>(null);
  const data = loaded?.reference.blockId === selected.blockId && loaded.reference.revision === selected.revision ? loaded : null;
  const [outline, setOutline] = useState<DocumentOutline | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [outlineError, setOutlineError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [moreLoading, setMoreLoading] = useState(false);
  const generation = useRef(0);
  const outlineGeneration = useRef(0);
  const selectedBlock = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const mine = ++outlineGeneration.current;
    setMoreLoading(false);
    setOutline(null);
    void loadDocumentOutline(reference).then((value) => { if (mine === outlineGeneration.current) { setOutline(value); setOutlineError(null); } }).catch((cause) => { if (mine === outlineGeneration.current) setOutlineError(formatUserError(t('citation.loadError'), cause)); });
    return () => { outlineGeneration.current += 1; };
  }, [reference, loadDocumentOutline, t, attempt]);

  useEffect(() => {
    const mine = ++generation.current;
    setLoading(true); setError(null);
    void loadEvidenceContext(selected).then((value) => { if (mine === generation.current) setData(value); })
      .catch((cause) => { if (mine === generation.current) { setData(null); setError(formatUserError(t('citation.loadError'), cause)); } })
      .finally(() => { if (mine === generation.current) setLoading(false); });
    return () => { generation.current += 1; };
  }, [selected, loadEvidenceContext, t, attempt]);

  useEffect(() => { if (!loading) selectedBlock.current?.scrollIntoView({ block: 'nearest' }); }, [data, loading]);

  const more = useCallback(async () => {
    if (moreLoading || !outline?.hasMore) return;
    const mine = outlineGeneration.current;
    setMoreLoading(true);
    try {
      const next = await loadDocumentOutline(reference, outline.nextIndex);
      if (mine !== outlineGeneration.current) return;
      setOutline((previous) => ({ ...next, sections: [...(previous?.sections ?? []), ...next.sections] }));
      setOutlineError(null);
    } catch (cause) { if (mine === outlineGeneration.current) setOutlineError(formatUserError(t('citation.loadError'), cause)); }
    finally { if (mine === outlineGeneration.current) setMoreLoading(false); }
  }, [moreLoading, outline, loadDocumentOutline, reference, t]);

  const target = data?.cards.find((card) => card.chunkId === selected.blockId);
  return <Modal open onClose={onClose} title={target?.documentTitle || t('citation.evidence')} surfaceClassName="!max-w-5xl !w-[min(96vw,72rem)]">
    <div className="space-y-3" data-testid="evidence-reader">
      {data && <div className="space-y-2">
        <div className="flex flex-wrap items-center gap-2 text-xs text-text-tertiary">
          <Badge>{evidenceLocationLabel(data.reference.locator, t)}</Badge>
          <span>{target?.sourceName}</span>
          <details className="min-w-0"><summary className="cursor-pointer">{t('citation.version')}</summary><p className="break-all font-mono text-[10px]">{data.reference.revision}</p></details>
        </div>
        {data.reference.status !== 'current' && <p role="status" className="rounded border border-warning/30 bg-warning/5 p-2 text-xs text-warning">{t(data.reference.status === 'missing' ? 'citation.fileRemoved' : 'citation.historical')}</p>}
        {data.reference.extractionMethod.includes('ocr') && <p className="text-xs text-text-tertiary">{t('citation.ocrText')}</p>}
        {data.reference.locator.kind === 'extracted' && <p className="text-xs text-warning">{t('citation.extractedText')}</p>}
        {data.reference.extractionMethod === 'native_cached_values' && <p className="text-xs text-text-tertiary">{t('citation.cachedValues')}</p>}
        {data.reference.extractionMethod === 'model_summary' && <p className="text-xs text-warning">{t('citation.modelSummary')}</p>}
        {data.truncated && <p className="text-xs text-warning">{t('preview.truncatedPreview')}</p>}
      </div>}
      {error && <div role="alert" className="text-sm text-danger">{error} <Button size="sm" variant="ghost" onClick={() => setAttempt((value) => value + 1)}>{t('common.retry')}</Button></div>}
      <div className="grid min-h-0 gap-3 md:grid-cols-[13rem_minmax(0,1fr)]">
        <nav aria-label={t('citation.contents')} className="max-h-40 overflow-y-auto rounded-lg border border-border p-2 md:max-h-[55vh]">
          <p className="mb-2 text-xs font-medium text-text-tertiary">{t('citation.contents')}</p>
          {outline?.sections.map((section) => <button key={`${section.reference.blockId}:${section.reference.contentHash}`} aria-current={section.reference.blockId === selected.blockId ? 'location' : undefined} onClick={() => setSelected(section.reference)} className="block w-full rounded px-2 py-1.5 text-left text-xs text-text-secondary hover:bg-surface-3 aria-[current=location]:bg-accent/10 aria-[current=location]:text-accent">{section.heading || evidenceLocationLabel(section.reference.locator, t)}</button>)}
          {outlineError && <p role="alert" className="text-xs text-danger">{outlineError} <button onClick={() => setAttempt((value) => value + 1)}>{t('common.retry')}</button></p>}
          {outline?.hasMore && <Button variant="ghost" size="sm" loading={moreLoading} onClick={() => void more()}>{t('citation.moreSections')}</Button>}
        </nav>
        <div className="max-h-[55vh] overflow-y-auto space-y-3" aria-busy={loading}>
          {loading ? <p role="status" className="flex gap-2 p-4 text-sm text-text-tertiary"><Loader2 size={16} className="animate-spin" />{t('common.loading')}</p> : data?.cards.map((card) => <div key={`${card.chunkId}:${card.evidenceRef?.contentHash}`} ref={card.chunkId === selected.blockId ? selectedBlock : undefined} data-testid={card.chunkId === selected.blockId ? 'selected-evidence-block' : 'adjacent-evidence-block'} className={`rounded-lg border p-3 ${card.chunkId === selected.blockId ? 'border-accent/50 bg-accent/5' : 'border-border bg-surface-1'}`}>
            <p className="mb-2 text-[11px] text-text-tertiary">{card.evidenceRef && evidenceLocationLabel(card.evidenceRef.locator, t)}</p>
            <p className="whitespace-pre-wrap break-words text-sm leading-relaxed text-text-secondary">{card.content}</p>
          </div>)}
        </div>
      </div>
      {target && <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border pt-2">
        <span className="min-w-0 break-all text-[10px] text-text-tertiary">{target.documentPath}</span>
        <Button size="sm" variant="secondary" icon={<ExternalLink size={12} />} disabled={data?.reference.status === 'missing'} onClick={() => {
          onClose();
          queueMicrotask(() => {
            if (isWebUrl(target.documentPath)) openWebLink(target.documentPath, target.documentTitle);
            else openFilePreview(target.documentPath, data?.reference.status === 'current' ? { locator: data.reference.locator, expectedHash: data.reference.documentHash, focusText: target.content } : undefined);
          });
        }}>{t('citation.openCurrent')}</Button>
      </div>}
    </div>
  </Modal>;
}
