export type FileType = "markdown" | "plaintext" | "log";

export interface Document {
  id: string;
  sourceId: string;
  path: string;
  title: string;
  contentHash: string;
  fileType: FileType;
  sizeBytes: number;
  createdAt: string;
  updatedAt: string;
  indexedAt: string | null;
}
