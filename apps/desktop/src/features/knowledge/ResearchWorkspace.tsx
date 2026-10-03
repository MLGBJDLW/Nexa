import { useEffect, useRef, useState } from 'react';
import { BookOpen, RefreshCw, Trash2 } from 'lucide-react';
import * as api from '../../lib/api';
import { useTranslation } from '../../i18n';
import { useProgress } from '../../lib/progressStore';
import { formatUserError } from '../../lib/userError';
import { useFilePreview } from '../preview/filePreviewContext';
import { Button } from '../../components/ui/Button';
import { Input } from '../../components/ui/Input';
import { Modal } from '../../components/ui/Modal';
import type { EvidenceCard } from '../../types/evidence';
import type { ResearchCell, ResearchDocument, ResearchSet, ResearchSetSummary } from '../../types/research';

let lastSetId = '';
let researchBasket: EvidenceCard[] = [];
export function ResearchWorkspace({ cards, sourceIds }: { cards: EvidenceCard[]; sourceIds: string[] }) {
  const { t } = useTranslation();
  const { openEvidence } = useFilePreview();
  const { knowledgeJobs } = useProgress();
  const job = knowledgeJobs.find(item => item.kind === 'research' && item.status === 'running');
  const finished = knowledgeJobs.find(item => item.kind === 'research' && item.status !== 'running');
  const [selected, setSelected] = useState<EvidenceCard[]>(researchBasket);
  const [title, setTitle] = useState('');
  const [questions, setQuestions] = useState('');
  const [summaries, setSummaries] = useState<ResearchSetSummary[]>([]);
  const [set, setSet] = useState<ResearchSet | null>(null);
  const [setId, setSetId] = useState(lastSetId);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [attempt, setAttempt] = useState(0);
  const [editing, setEditing] = useState<{ document: ResearchDocument; cell: ResearchCell; note: string; state: ResearchCell['reviewState'] } | null>(null);
  const generation = useRef(0);
  const disabled = busy || Boolean(job);
  const documentCards = [...new Map(cards.filter(card => card.evidenceRef).map(card => [card.documentId, card])).values()];

  useEffect(() => { setSelected(previous => previous.filter(card => !sourceIds.length || sourceIds.includes(card.sourceId))); }, [sourceIds]);
  useEffect(() => { researchBasket = selected; }, [selected]);
  useEffect(() => {
    let active = true;
    void api.listResearchSets().then(value => { if (active) setSummaries(value ?? []); }).catch(cause => { if (active) setError(formatUserError(t('knowledge.researchLoadError'), cause)); });
    return () => { active = false; };
  }, [attempt, finished?.revision, finished?.id, t]);
  useEffect(() => {
    const mine = ++generation.current; lastSetId = setId;
    setSet(null); setEditing(null); setError('');
    if (!setId) { setLoading(false); return; }
    setLoading(true);
    void api.getResearchSet(setId).then(value => { if (mine === generation.current) setSet(value); }).catch(cause => { if (mine === generation.current) setError(formatUserError(t('knowledge.researchLoadError'), cause)); }).finally(() => { if (mine === generation.current) setLoading(false); });
    return () => { generation.current++; };
  }, [setId, attempt, finished?.revision, finished?.id, t]);

  async function mutate(action: () => Promise<ResearchSet | null>) {
    const mine = generation.current;
    setBusy(true); setError('');
    try { const value = await action(); if (mine !== generation.current) return; if (value) { setSet(value); setSetId(value.summary.id); } setEditing(null); setAttempt(count => count + 1); }
    catch (cause) { if (mine === generation.current) setError(formatUserError(t('knowledge.researchLoadError'), cause)); }
    finally { setBusy(false); }
  }
  const status = (cell: ResearchCell) => cell.stale ? t('knowledge.researchStale') : t(`knowledge.researchState_${cell.reviewState}`);
  return <details className="mb-4 rounded-xl border border-border bg-surface-1 p-3" data-testid="research-workspace">
    <summary className="cursor-pointer text-sm font-medium text-text-secondary">{t('knowledge.researchTitle')} {selected.length ? `(${selected.length}/8)` : ''}</summary>
    <div className="mt-3 space-y-3">
      <p className="text-xs text-text-tertiary">{t('knowledge.researchDescription')}</p>
      <label className="block text-xs text-text-secondary">{t('knowledge.researchSaved')}<select aria-label={t('knowledge.researchSaved')} value={setId} disabled={disabled} onChange={event => setSetId(event.target.value)} className="mt-1 h-9 w-full rounded border border-border bg-surface-0 px-2">
        <option value="">{t('knowledge.researchNew')}</option>{summaries.map(item => <option key={item.id} value={item.id}>{item.title}</option>)}
      </select></label>
      {error && <p role="alert" className="text-xs text-danger">{error} <button onClick={() => setAttempt(value => value + 1)}>{t('common.retry')}</button></p>}
      {job && <p role="status" className="text-xs text-accent">{t('common.loading')} {String(job.progress.current ?? 0)}/{String(job.progress.total ?? 0)}</p>}
      {finished && (finished.status === 'failed' || finished.status === 'interrupted') && !job && <p role="status" className="text-xs text-warning">{t('knowledge.researchInterrupted')} {finished.error === 'runtime_interrupted' ? '' : finished.error}</p>}
      {!setId && <>
        <div className="max-h-48 space-y-1 overflow-auto rounded border border-border p-2">
          {documentCards.length === 0 && <p className="text-xs text-text-tertiary">{t('knowledge.researchSelectHint')}</p>}
          {documentCards.map(card => <label key={card.documentId} className="flex items-start gap-2 rounded p-1 text-xs text-text-secondary"><input type="checkbox" checked={selected.some(item => item.documentId === card.documentId)} disabled={disabled || (selected.length >= 8 && !selected.some(item => item.documentId === card.documentId))} onChange={event => setSelected(previous => event.target.checked ? [...previous.filter(item => item.documentId !== card.documentId), card] : previous.filter(item => item.documentId !== card.documentId))} /><span className="break-all">{card.documentTitle} <span className="text-text-tertiary">{card.sourceName}</span></span></label>)}
        </div>
        {selected.length > 0 && <div className="flex flex-wrap gap-1">{selected.map(card => <button key={card.documentId} disabled={disabled} onClick={() => setSelected(previous => previous.filter(item => item.documentId !== card.documentId))} className="rounded bg-accent/10 px-2 py-1 text-xs text-accent" aria-label={`${t('common.remove')} ${card.documentTitle}`}>{card.documentTitle} ×</button>)}</div>}
        <label className="block text-xs text-text-secondary">{t('knowledge.researchName')}<Input aria-label={t('knowledge.researchName')} value={title} maxLength={120} disabled={disabled} onChange={event => setTitle(event.target.value)} /></label>
        <label className="block text-xs text-text-secondary">{t('knowledge.researchQuestions')}<textarea aria-label={t('knowledge.researchQuestions')} rows={3} maxLength={2406} value={questions} disabled={disabled} onChange={event => setQuestions(event.target.value)} className="mt-1 w-full rounded border border-border bg-surface-0 p-2" /></label>
        <Button size="sm" disabled={disabled || !selected.length || !title.trim() || !questions.trim()} loading={busy} onClick={() => void mutate(() => api.createResearchSet({title,questions:questions.split('\n').map(item => item.trim()).filter(Boolean),documents:selected.flatMap(card => card.evidenceRef ? [card.evidenceRef] : [])}))}>{t('knowledge.researchCreate')}</Button>
      </>}
      {loading && <p role="status" className="text-xs text-text-tertiary">{t('common.loading')}</p>}
      {set && <>
        <div className="flex flex-wrap items-center gap-2"><Button size="sm" disabled={disabled} loading={busy} icon={<RefreshCw size={13} />} onClick={() => void mutate(() => api.refreshResearchSet(set.summary.id))}>{t('knowledge.researchRefresh')}</Button>
          <Button size="sm" variant="ghost" disabled={disabled} icon={<Trash2 size={13} />} onClick={() => void mutate(async () => { await api.deleteResearchSet(set.summary.id); setSetId(''); return null; })}>{t('common.delete')}</Button>
          <span className="text-xs text-text-tertiary">{t('knowledge.researchBudget')}</span></div>
        <div className="max-h-[60vh] overflow-auto rounded border border-border" tabIndex={0} aria-label={t('knowledge.researchTitle')}>
          <table className="w-full border-collapse text-xs"><thead className="sticky top-0 bg-surface-2"><tr><th className="min-w-36 p-3 text-left">{t('knowledge.researchDocument')}</th>{set.questions.map((question, index) => <th key={index} className="min-w-64 max-w-80 p-3 text-left">{question}</th>)}</tr></thead>
            <tbody>{set.documents.map(document => <tr key={document.reference.documentId} className="border-t border-border"><th className="p-3 align-top text-left"><p className="max-w-44 break-words font-medium">{document.title}</p><button className="mt-2 text-accent" onClick={() => openEvidence?.(document.reference)}>{t('citation.evidence')}</button></th>{document.cells.map(cell => <td key={cell.questionIndex} className="max-w-80 border-l border-border p-3 align-top">
              <p className={cell.stale ? 'font-medium text-warning' : 'font-medium text-text-secondary'}>{status(cell)}</p>
              {cell.note && <p className="mt-2 whitespace-pre-wrap break-words text-text-secondary">{cell.note}</p>}
              {cell.evidence.map(card => <button key={card.chunkId} onClick={() => card.evidenceRef && openEvidence?.(card.evidenceRef)} className="mt-2 block w-full rounded border border-border p-2 text-left text-text-tertiary hover:border-accent"><BookOpen size={12} className="mb-1" /><span className="line-clamp-4 break-words">{card.content}</span></button>)}
              <button disabled={disabled || cell.stale || cell.reviewState === 'pending'} onClick={() => setEditing({document,cell,note:cell.note,state:cell.reviewState})} className="mt-2 text-accent disabled:opacity-40">{t('knowledge.researchReview')}</button>
            </td>)}</tr>)}</tbody></table>
          {!set.documents.length && <p className="p-3 text-xs text-text-tertiary">{t('knowledge.researchSourcesRemoved')}</p>}
        </div>
      </>}
    </div>
    {editing && set && <Modal open onClose={() => { if (!busy) setEditing(null); }} title={t('knowledge.researchReview')}>
      <p className="mb-3 text-sm">{set.questions[editing.cell.questionIndex]}</p>
      <label className="block text-xs">{t('knowledge.researchConclusion')}<textarea rows={5} value={editing.note} disabled={busy} maxLength={4000} onChange={event => setEditing({...editing,note:event.target.value})} className="mt-1 w-full rounded border border-border bg-surface-1 p-2" /></label>
      <select aria-label={t('knowledge.researchReview')} value={editing.state} disabled={busy} onChange={event => setEditing({...editing,state:event.target.value as ResearchCell['reviewState']})} className="my-3 h-9 w-full rounded border border-border bg-surface-1 px-2">{(['needs_review','supported','not_found','conflict'] as const).map(state => <option key={state} value={state}>{t(`knowledge.researchState_${state}`)}</option>)}</select>
      <Button size="sm" loading={busy} onClick={() => void mutate(() => api.reviewResearchCell({setId:set.summary.id,documentId:editing.document.reference.documentId,questionIndex:editing.cell.questionIndex,expectedRevision:set.summary.revision,reviewState:editing.state,note:editing.note}))}>{t('common.save')}</Button>
    </Modal>}
  </details>;
}
