export interface SubagentToolDescriptor {
  name: string;
  label: string;
  description: string;
  enabledByDefault: boolean;
}

export const SUBAGENT_TOOL_CATALOG: SubagentToolDescriptor[] = [
  {
    name: 'search_knowledge_base',
    label: 'Knowledge Search',
    description: 'Search indexed local knowledge and find candidate evidence.',
    enabledByDefault: true,
  },
  {
    name: 'read_file',
    label: 'Read File',
    description: 'Open and inspect files that belong to registered sources.',
    enabledByDefault: true,
  },
  {
    name: 'retrieve_evidence',
    label: 'Retrieve Evidence',
    description: 'Load exact evidence chunks and summarize or verify findings.',
    enabledByDefault: true,
  },
  {
    name: 'list_sources',
    label: 'List Sources',
    description: 'Show available indexed sources.',
    enabledByDefault: true,
  },
  {
    name: 'list_documents',
    label: 'List Documents',
    description: 'Browse indexed documents in the knowledge base.',
    enabledByDefault: true,
  },
  {
    name: 'list_dir',
    label: 'List Directory',
    description: 'Inspect source directories and candidate file paths.',
    enabledByDefault: true,
  },
  {
    name: 'get_chunk_context',
    label: 'Chunk Context',
    description: 'Expand truncated chunk snippets with nearby context.',
    enabledByDefault: true,
  },
  {
    name: 'fetch_url',
    label: 'Fetch URL',
    description: 'Load a web page when the delegated task needs external context.',
    enabledByDefault: true,
  },
  {
    name: 'search_playbooks',
    label: 'Search Playbooks',
    description: 'Inspect saved playbooks and reusable workflows.',
    enabledByDefault: true,
  },
  {
    name: 'get_document_info',
    label: 'Document Info',
    description: 'Inspect document metadata and indexing details.',
    enabledByDefault: true,
  },
  {
    name: 'compare',
    label: 'Compare',
    description: 'Compare passages, candidates, or drafts side by side.',
    enabledByDefault: true,
  },
  {
    name: 'get_statistics',
    label: 'Statistics',
    description: 'Inspect corpus statistics and aggregate counts.',
    enabledByDefault: true,
  },
  {
    name: 'date_search',
    label: 'Date Search',
    description: 'Find content constrained by date-related filters.',
    enabledByDefault: true,
  },
  {
    name: 'summarize_document',
    label: 'Summarize Document',
    description: 'Generate document-level summaries for large files.',
    enabledByDefault: true,
  },
  {
    name: 'update_plan',
    label: 'Update Plan',
    description: 'Record a structured plan artifact for the delegated task.',
    enabledByDefault: true,
  },
  {
    name: 'record_verification',
    label: 'Record Verification',
    description: 'Record structured verification checks before returning.',
    enabledByDefault: true,
  },
];

export const DEFAULT_SUBAGENT_TOOL_NAMES = SUBAGENT_TOOL_CATALOG
  .filter(tool => tool.enabledByDefault)
  .map(tool => tool.name);

export function getSubagentToolDescriptor(name: string) {
  return SUBAGENT_TOOL_CATALOG.find(tool => tool.name === name) ?? null;
}

export function normalizeSubagentToolSelection(selection: string[] | null | undefined): string[] {
  const selected = new Set(selection ?? DEFAULT_SUBAGENT_TOOL_NAMES);
  return SUBAGENT_TOOL_CATALOG
    .filter(tool => selected.has(tool.name))
    .map(tool => tool.name);
}

export function usesDefaultSubagentToolSelection(selection: string[] | null | undefined): boolean {
  const normalized = normalizeSubagentToolSelection(selection);
  if (normalized.length !== DEFAULT_SUBAGENT_TOOL_NAMES.length) return false;
  return normalized.every((name, index) => name === DEFAULT_SUBAGENT_TOOL_NAMES[index]);
}
