import { invoke } from "@tauri-apps/api/core";
import type {
  Source,
  EvidenceCard,
  Playbook,
  PlaybookCitation,
  SearchResult,
  IngestResult,
  IndexStats,
  QueryLog,
  Feedback,
  EmbedResult,
} from "../types";

// ── Sources ─────────────────────────────────────────────────────────────

export const addSource = (
  rootPath: string,
  includeGlobs: string[],
  excludeGlobs: string[],
) => invoke<Source>("add_source", { kind: "local_folder", rootPath, includeGlobs, excludeGlobs });

export const listSources = () => invoke<Source[]>("list_sources");

export const deleteSource = (sourceId: string) =>
  invoke<void>("delete_source", { sourceId });

export const scanSource = (sourceId: string) =>
  invoke<IngestResult>("scan_source", { sourceId });

export const scanAllSources = () =>
  invoke<IngestResult[]>("scan_all_sources");

// ── Search ──────────────────────────────────────────────────────────────

export const search = (queryText: string, limit?: number, offset?: number) =>
  invoke<SearchResult>("search", { queryText, limit, offset });

export const getEvidenceCard = (chunkId: string) =>
  invoke<EvidenceCard>("get_evidence_card", { chunkId });

// ── Index ───────────────────────────────────────────────────────────────

export const getIndexStats = () => invoke<IndexStats>("get_index_stats");

export const rebuildIndex = () => invoke<void>("rebuild_index");

// ── Playbooks ───────────────────────────────────────────────────────────

export const createPlaybook = (
  title: string,
  description: string,
  queryText: string,
) => invoke<Playbook>("create_playbook", { title, description, queryText });

export const listPlaybooks = () => invoke<Playbook[]>("list_playbooks");

export const getPlaybook = (playbookId: string) =>
  invoke<Playbook>("get_playbook", { playbookId });

export const updatePlaybook = (
  playbookId: string,
  title: string,
  description: string,
) => invoke<Playbook>("update_playbook", { playbookId, title, description });

export const deletePlaybook = (playbookId: string) =>
  invoke<void>("delete_playbook", { playbookId });

export const addCitation = (
  playbookId: string,
  chunkId: string,
  note: string,
  sortOrder: number,
) => invoke<PlaybookCitation>("add_citation", { playbookId, chunkId, note, sortOrder });

export const listCitations = (playbookId: string) =>
  invoke<PlaybookCitation[]>("list_citations", { playbookId });

export const removeCitation = (citationId: string) =>
  invoke<void>("remove_citation", { citationId });

// ── Query Log ───────────────────────────────────────────────────────────

export const getRecentQueries = (limit?: number) =>
  invoke<QueryLog[]>("get_recent_queries", { limit });

// ── Hybrid Search ───────────────────────────────────────────────────────

export const hybridSearch = (queryText: string) =>
  invoke<SearchResult>('hybrid_search', { queryText });

// ── Embeddings ──────────────────────────────────────────────────────────

export const embedSource = (sourceId: string) =>
  invoke<EmbedResult>('embed_source', { sourceId });

export const rebuildEmbeddings = () =>
  invoke<EmbedResult>('rebuild_embeddings');

// ── Feedback ────────────────────────────────────────────────────────────

export const addFeedback = (chunkId: string, queryText: string, action: string) =>
  invoke<Feedback>('add_feedback', { chunkId, queryText, action });

export const getFeedbackForQuery = (queryText: string) =>
  invoke<Feedback[]>('get_feedback_for_query', { queryText });

export const deleteFeedback = (feedbackId: string) =>
  invoke<void>('delete_feedback', { feedbackId });
