import { buildRelationBundles } from '../src/lib/knowledgeGraphRelations';
import { buildGraphAgentContext, buildGraphCollectionContext } from '../src/lib/knowledgeGraphAgent';
import type { KnowledgeGraphEdge, KnowledgeGraphNode } from '../src/types/knowledge';

function check(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

function edge(id: string, source: string, target: string, relationType: string): KnowledgeGraphEdge {
  return { id, source, target, relationType, strength: 0.8, evidenceDocId: 'doc', evidenceTitle: 'Evidence', evidencePath: '/evidence.md' };
}

const manyEdges = Array.from({ length: 8000 }, (_, i) => edge(String(i), `a${i}`, `b${i}`, 'causes'));
const start = performance.now();
check(buildRelationBundles(manyEdges).length === 8000, 'Every independent pair must survive aggregation');
console.log(`graph aggregation: 8000 pairs in ${(performance.now() - start).toFixed(1)} ms`);

const incoming = edge('incoming', 'b', 'a', 'causes');
const cooccurrence = { ...edge('shared', 'a', 'b', 'co_occurs'), evidenceSource: 'cooccurrence' };
const mixed = buildRelationBundles([cooccurrence, incoming])[0];
check(mixed.direction === 'directed', 'Co-occurrence must not turn an incoming cause into a bidirectional relation');
check(mixed.source === 'b' && mixed.target === 'a', 'A directed bundle must point in the direction of its directed evidence');
check(buildRelationBundles([incoming, edge('outgoing', 'a', 'b', 'enables')])[0].direction === 'bidirectional', 'Two directed orientations remain bidirectional');
check(buildRelationBundles([cooccurrence])[0].direction === 'undirected', 'Shared evidence is undirected');
check(buildRelationBundles([edge('x', 'a::b', 'c', 'causes'), edge('y', 'a', 'b::c', 'causes')]).length === 2, 'Pair lookup must not collide on separators');

const node: KnowledgeGraphNode = { id: 'a', label: 'Alpha', entityType: 'concept', description: '', mentionCount: 1, documentCount: 1, linkCount: 2, firstSeenDoc: 'doc', documents: [] };
const context = buildGraphAgentContext({ sourceId: null, sourceLabel: null, pathPrefix: null, scopeLabel: null, node, edges: [incoming, cooccurrence], nodeLabelById: new Map([['a', 'Alpha'], ['b', 'Beta']]) });
const prompt = buildGraphCollectionContext(context)!.queryText ?? '';
check(prompt.includes('Beta [b] --causes--> Alpha [a]'), 'Agent context must preserve incoming subject, predicate and object');
check(!prompt.includes('Alpha --causes'), 'Focusing the target must never reverse a fact');
check(prompt.includes('cooccurrence'), 'Agent context retains the evidence origin');
console.log('ok - graph bundles and agent context preserve relation semantics');
