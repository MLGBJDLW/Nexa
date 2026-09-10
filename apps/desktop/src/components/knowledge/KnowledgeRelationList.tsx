import { useTranslation } from '../../i18n';
import type { KnowledgeGraphNode } from '../../types/knowledge';
import { isUndirectedRelationType, relationArrow, type KnowledgeGraphRelationBundle } from '../../lib/knowledgeGraphRelations';
import { Badge } from '../ui/Badge';
import { FileBadge } from '../ui/FileBadge';

/** The same graph facts as the canvas, in reading order with evidence one click away. */
export function KnowledgeRelationList({ bundles, nodes, selectedId, onSelect }: {
  bundles: KnowledgeGraphRelationBundle[];
  nodes: Map<string, KnowledgeGraphNode>;
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="h-[562px] overflow-y-auto p-4" data-testid="knowledge-relation-list">
      <p className="mb-4 text-xs leading-5 text-text-tertiary">{t('knowledge.readingHint')}</p>
      <div className="space-y-3">
        {bundles.map((bundle) => (
          <article key={bundle.id} className={`overflow-hidden rounded-lg border ${selectedId === bundle.id ? 'border-accent/60 bg-accent/5' : 'border-border bg-surface-0/60'}`}>
            <button type="button" onClick={() => onSelect(bundle.id)} aria-pressed={selectedId === bundle.id} className="flex w-full flex-wrap items-center gap-2 border-b border-border/60 px-4 py-3 text-left text-sm font-semibold text-text-primary hover:bg-surface-2">
              <span>{nodes.get(bundle.source)?.label ?? bundle.source}</span>
              <span className="text-accent">{relationArrow(bundle.direction)}</span>
              <span>{nodes.get(bundle.target)?.label ?? bundle.target}</span>
              <Badge variant="muted" className="ml-auto">{bundle.relationCount} {t('knowledge.edges')}</Badge>
            </button>
            <ul className="divide-y divide-border/50 px-4">
              {bundle.edges.map((edge) => (
                <li key={edge.id} className="py-3">
                  <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
                    <span className="font-medium text-text-primary">{nodes.get(edge.source)?.label ?? edge.source}</span>
                    <span className="text-accent">{edge.relationType === 'co_occurs' ? t('knowledge.relationType.coOccurs') : edge.relationType.replace(/_/g, ' ')}</span>
                    <span className="text-text-tertiary">{isUndirectedRelationType(edge.relationType) ? '—' : '→'}</span>
                    <span className="font-medium text-text-primary">{nodes.get(edge.target)?.label ?? edge.target}</span>
                  </div>
                  <div className="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-text-tertiary">
                    <Badge variant={edge.evidenceSource === 'cooccurrence' ? 'muted' : 'info'}>{t(edge.evidenceSource === 'cooccurrence' ? 'knowledge.sharedEvidence' : 'knowledge.explicitEvidence')}</Badge>
                    <span>{t('knowledge.strongestStrength')}: {edge.strength.toFixed(2)}</span>
                    {edge.evidencePath ? <FileBadge path={edge.evidencePath} /> : <span>{edge.evidenceTitle ?? t('knowledge.noEvidenceDocuments')}</span>}
                  </div>
                  {edge.evidenceSnippet && <blockquote className="mt-2 border-l-2 border-accent/25 pl-3 text-xs leading-5 text-text-secondary">{edge.evidenceSnippet}</blockquote>}
                </li>
              ))}
            </ul>
          </article>
        ))}
        {bundles.length === 0 && <p className="py-8 text-center text-sm text-text-tertiary">{t('knowledge.noRelations')}</p>}
      </div>
    </div>
  );
}
