import type { KnowledgeGraphEdge } from '../types/knowledge';

export type RelationCategory = 'conflict' | 'causal' | 'hierarchy' | 'event' | 'social' | 'general';
export type RelationDirection = 'directed' | 'bidirectional' | 'undirected';

export interface KnowledgeGraphRelationBundle {
  id: string;
  source: string;
  target: string;
  edges: KnowledgeGraphEdge[];
  edgeIds: string[];
  relationTypes: string[];
  relationCount: number;
  direction: RelationDirection;
  category: RelationCategory;
  strongestStrength: number;
  averageStrength: number;
  evidenceTitles: string[];
}

function relationMatches(value: string, needles: string[]) {
  const normalized = value.toLowerCase();
  return needles.some((needle) => normalized.includes(needle));
}

export function relationCategory(relationTypes: string[]): RelationCategory {
  if (relationTypes.some((value) => relationMatches(value, ['conflict', 'enemy', 'rival', 'threat', 'oppose', 'contradict']))) {
    return 'conflict';
  }
  if (relationTypes.some((value) => relationMatches(value, ['cause', 'lead', 'affect', 'influence', 'enable', 'prevent', 'trigger']))) {
    return 'causal';
  }
  if (relationTypes.some((value) => relationMatches(value, ['parent', 'child', 'belongs', 'member', 'part', 'located', 'contains', 'owns']))) {
    return 'hierarchy';
  }
  if (relationTypes.some((value) => relationMatches(value, ['event', 'appears', 'participates', 'occurs', 'incident', 'meeting']))) {
    return 'event';
  }
  if (relationTypes.some((value) => relationMatches(value, ['friend', 'ally', 'mentor', 'family', 'knows', 'protect', 'trust', 'love']))) {
    return 'social';
  }
  return 'general';
}

export function isUndirectedRelationType(relationType: string) {
  const normalized = relationType.toLowerCase();
  return (
    normalized.includes('related') ||
    normalized.includes('similar') ||
    normalized.includes('co_occurs') ||
    normalized.includes('cooccurs') ||
    normalized.includes('cooccurrence') ||
    normalized.includes('associated')
  );
}

export function buildRelationBundles(edges: KnowledgeGraphEdge[]): KnowledgeGraphRelationBundle[] {
  const bundles: KnowledgeGraphRelationBundle[] = [];
  const pairIndex = new Map<string, KnowledgeGraphRelationBundle>();

  for (const edge of edges) {
    // Tuple encoding prevents IDs containing separators from aliasing another pair.
    const key = JSON.stringify(edge.source <= edge.target ? [edge.source, edge.target] : [edge.target, edge.source]);
    let bundle = pairIndex.get(key);
    if (!bundle) {
      bundle = {
        id: `${edge.source}::${edge.target}`,
        source: edge.source,
        target: edge.target,
        edges: [],
        edgeIds: [],
        relationTypes: [],
        relationCount: 0,
        direction: 'directed',
        category: 'general',
        strongestStrength: 0,
        averageStrength: 0,
        evidenceTitles: [],
      };
      bundles.push(bundle);
      pairIndex.set(key, bundle);
    }

    bundle.edges.push(edge);
    bundle.edgeIds.push(edge.id);
  }

  for (const bundle of bundles) {
    const directedEdges = bundle.edges.filter((edge) => !isUndirectedRelationType(edge.relationType));
    if (directedEdges.length > 0) {
      bundle.source = directedEdges[0].source;
      bundle.target = directedEdges[0].target;
    }
    const hasReverse = directedEdges.some((edge) => edge.source !== bundle.source);
    let totalStrength = 0;
    const types = new Set<string>();
    const titles = new Set<string>();
    for (const edge of bundle.edges) {
      types.add(edge.relationType);
      for (const title of edge.evidenceTitles ?? []) titles.add(title);
      if (edge.evidenceTitle) titles.add(edge.evidenceTitle);
      totalStrength += edge.strength;
      bundle.strongestStrength = Math.max(bundle.strongestStrength, edge.strength);
    }
    bundle.relationTypes = [...types].sort((a, b) => a.localeCompare(b));
    bundle.evidenceTitles = [...titles];
    bundle.relationCount = bundle.edges.length;
    bundle.direction = directedEdges.length === 0 ? 'undirected' : hasReverse ? 'bidirectional' : 'directed';
    bundle.category = relationCategory(bundle.relationTypes);
    bundle.averageStrength = bundle.edges.length > 0 ? totalStrength / bundle.edges.length : 0;
  }

  return bundles.sort((a, b) => {
    const countDelta = b.relationCount - a.relationCount;
    if (countDelta !== 0) return countDelta;
    const strengthDelta = b.strongestStrength - a.strongestStrength;
    if (strengthDelta !== 0) return strengthDelta;
    return a.id.localeCompare(b.id);
  });
}

export function relationArrow(direction: RelationDirection): string {
  return direction === 'directed' ? '→' : direction === 'bidirectional' ? '↔' : '—';
}
