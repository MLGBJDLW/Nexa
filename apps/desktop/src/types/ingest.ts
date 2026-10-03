export interface IngestResult {
  sourceId: string;
  filesScanned: number;
  filesAdded: number;
  filesUpdated: number;
  filesSkipped: number;
  filesFailed: number;
  errors: string[];
}

export interface KnowledgeJob {
  id: string;
  kind: 'scan' | 'scan-all' | 'embed' | 'rebuild-embeddings' | 'compile' | 'research';
  sourceId: string | null;
  status: 'running' | 'completed' | 'failed' | 'interrupted';
  progress: Partial<BatchProgress> & { documentId?: string; documentTitle?: string | null };
  error: string | null;
  revision: number;
  startedAt: string;
  finishedAt: string | null;
}

export interface SourceIndexHealth {
  sourceId: string;
  documents: number;
  chunks: number;
  keywordChunks: number;
  embeddedChunks: number;
  compiledDocuments: number;
  staleDocuments: number;
  partialDocuments: number;
  parseWarnings: number;
  failedFiles: number;
  needsReparse: number;
  embeddingSpace: string;
  lastScan: IngestResult | null;
}

export interface ScanProgress {
  sourceId: string;
  phase: string;
  current: number;
  total: number;
  currentFile: string | null;
}

export interface BatchProgress {
  operation: string;
  sourceIndex: number;
  sourceCount: number;
  sourceId: string;
  phase: string;
  current: number;
  total: number;
  currentFile: string | null;
}

export interface FtsProgress {
  operation: string;
  phase: string;
}

export interface DownloadProgress {
  filename: string;
  bytesDownloaded: number;
  totalBytes: number | null;
  fileIndex: number;
  totalFiles: number;
}
