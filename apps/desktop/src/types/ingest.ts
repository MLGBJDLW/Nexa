export interface IngestResult {
  sourceId: string;
  filesScanned: number;
  filesAdded: number;
  filesUpdated: number;
  filesSkipped: number;
  filesFailed: number;
  errors: string[];
}
