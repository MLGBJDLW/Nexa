import type { EvidenceCard, EvidenceRef } from './evidence';
export interface ResearchSetSummary { id: string; title: string; revision: number; updatedAt: string }
export interface ResearchCell { questionIndex: number; reviewState: 'pending' | 'needs_review' | 'supported' | 'not_found' | 'conflict'; note: string; stale: boolean; evidence: EvidenceCard[] }
export interface ResearchDocument { reference: EvidenceRef; title: string; path: string; cells: ResearchCell[] }
export interface ResearchSet { summary: ResearchSetSummary; questions: string[]; documents: ResearchDocument[] }
