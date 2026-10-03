export interface Highlight {
  start: number;
  end: number;
  term: string;
}

export interface EvidenceCard {
  evidenceRef?: EvidenceRef;
  chunkId: string;
  documentId: string;
  sourceId: string;
  sourceName: string;
  documentPath: string;
  documentTitle: string;
  chunkIndex?: number;
  chunkKind?: string;
  content: string;
  headingPath: string[];
  score: number;
  highlights: Highlight[];
  snippet?: string;
  documentDate?: string;
  credibility?: number;
  freshnessDays?: number;
}

export type EvidenceLocator =
  | { kind: 'text'; byteStart: number; byteEnd: number; lineStart: number; lineEnd: number }
  | { kind: 'pdf'; page: number; bbox?: [number, number, number, number] }
  | { kind: 'document'; part: string; paragraph: number; table?: number; row?: number; column?: number }
  | { kind: 'sheet'; sheet: string; range: string }
  | { kind: 'slide'; slide: number }
  | { kind: 'media'; startMs: number; endMs: number }
  | { kind: 'extracted'; section: string }
  | { kind: 'unknown' };

export interface EvidenceRef {
  sourceId: string;
  documentId: string;
  revision: string;
  documentHash: string;
  blockId: string;
  contentHash: string;
  locator: EvidenceLocator;
  extractionMethod: string;
  status: 'current' | 'historical' | 'missing';
}

export interface EvidenceContext { reference: EvidenceRef; cards: EvidenceCard[]; truncated: boolean }
export interface DocumentSection { reference: EvidenceRef; chunkIndex: number; heading: string | null }
export interface DocumentOutline { sections: DocumentSection[]; hasMore: boolean; nextIndex: number | null }
