import { memo } from 'react';
import { AlertCircle, ChevronRight, FilePenLine, Loader2 } from 'lucide-react';
import { useTranslation } from '../../i18n';
import type { ToolCallEvent } from '../../lib/streaming/protocol';
import { isPendingToolCallStatus, isUnsuccessfulToolCallStatus } from '../../lib/streaming/toolStatus';
import { DiffStatsTicker } from './DiffStatsTicker';
import { diffPathAliasKeys, extractDiffStatsArtifact, extractFileDiffArtifacts } from './FileDiffPreview';
import { TimelineToolCallCard } from './ToolCallCard';

export interface FileToolGroup {
  path: string;
  calls: ToolCallEvent[];
}

/** Presentation only: keep every invocation and its receipt, but give one file
 * one stable row for the current user turn. Never merge paths by basename. */
export function groupFileToolCalls(calls: ToolCallEvent[]): Map<ToolCallEvent, FileToolGroup> {
  const groups = new Map<string, FileToolGroup>();
  const byCall = new Map<ToolCallEvent, FileToolGroup>();
  const records: { call: ToolCallEvent; path: string; absolute?: string; aliases: string[] }[] = [];
  const absoluteByAlias = new Map<string, Set<string>>();
  const keyFor = (path: string) => { const keys = diffPathAliasKeys(path); return keys[keys.length - 1] ?? ''; };
  for (const call of calls) {
    if (!['edit_file', 'create_file', 'write_note'].includes(call.toolName)) continue;
    const diffs = extractFileDiffArtifacts(call.artifacts);
    if (diffs.length > 1) continue;
    const artifacts = call.artifacts as Record<string, unknown> | undefined;
    const rawDiff = artifacts?.diff as { path?: string; absolutePath?: string } | undefined;
    const changes = Array.isArray(artifacts?.fileChanges) ? artifacts.fileChanges : [];
    if (changes.length > 1) continue;
    const receipt = changes[0] as { path?: string; absolutePath?: string } | undefined;
    let argumentPath: string | undefined;
    try {
      const args = JSON.parse(call.arguments);
      if (typeof args.path === 'string') argumentPath = args.path;
    } catch { /* Incomplete arguments wait for a semantic path preview. */ }
    const candidates = [diffs[0]?.absolutePath, receipt?.absolutePath, rawDiff?.absolutePath,
      diffs[0]?.path, receipt?.path, rawDiff?.path, argumentPath].filter((path): path is string => typeof path === 'string' && Boolean(path.trim()));
    const path = candidates[0];
    if (!path) continue;
    const absolute = candidates.find(path => /^(?:[a-z]:[\\/]|\/|\\\\)/i.test(path));
    const aliases = [...new Set(candidates.map(keyFor).filter(Boolean))];
    records.push({ call, path, absolute, aliases });
    if (absolute) for (const alias of aliases) {
      const targets = absoluteByAlias.get(alias) ?? new Set<string>();
      targets.add(keyFor(absolute));
      absoluteByAlias.set(alias, targets);
    }
  }
  for (const { call, path, absolute, aliases } of records) {
    const known = aliases.map(alias => absoluteByAlias.get(alias)).find(targets => targets?.size === 1);
    const key = absolute ? keyFor(absolute) : known?.values().next().value ?? keyFor(path);
    if (!key) continue;
    let group = groups.get(key);
    if (!group) {
      group = { path, calls: [] };
      groups.set(key, group);
    }
    if (!byCall.has(call)) group.calls.push(call);
    byCall.set(call, group);
  }
  return byCall;
}

export const FileChangeTrace = memo(function FileChangeTrace({ group, conversationId, parentRunActive }: {
  group: FileToolGroup;
  conversationId?: string | null;
  parentRunActive: boolean;
}) {
  const { t } = useTranslation();
  const pending = parentRunActive && group.calls.some(call => isPendingToolCallStatus(call.status));
  const failed = group.calls.some(call => call.isError || isUnsuccessfulToolCallStatus(call.status));
  // These are cumulative operation counts; the existing file-changes dock
  // remains the authority for the net on-disk diff and baseline readback.
  const stats = group.calls.map(call => call.isError || isUnsuccessfulToolCallStatus(call.status)
    ? null : extractDiffStatsArtifact(call.artifacts));
  const additions = stats.reduce((total, item) => total + (item?.additions ?? 0), 0);
  const deletions = stats.reduce((total, item) => total + (item?.deletions ?? 0), 0);
  const Icon = pending ? Loader2 : failed ? AlertCircle : FilePenLine;
  return <details className="group/file-changes my-1 min-w-0 rounded-lg border border-border bg-surface-1/50" data-testid="file-change-trace" data-file-path={group.path}>
    <summary className="flex min-h-9 cursor-pointer list-none items-center gap-2 px-3 py-2 text-xs text-text-secondary" aria-busy={pending}>
      <ChevronRight size={13} className="shrink-0 transition-transform group-open/file-changes:rotate-90 motion-reduce:transition-none" />
      <Icon size={14} className={`${pending ? 'animate-spin motion-reduce:animate-none' : ''} ${failed ? 'text-amber-500' : 'text-accent'} shrink-0`} />
      <span className="min-w-0 flex-1 truncate font-medium text-text-primary" title={group.path}>{group.path}</span>
      <span className="shrink-0">{t('chat.workflowTasks', { count: String(group.calls.length) })}</span>
      {failed && <span className="text-amber-500">{t('chat.toolBriefError')}</span>}
      {stats.some(Boolean) && <DiffStatsTicker additions={additions} deletions={deletions} showFiles={false} showReplacements={false} compact live={pending} testIdPrefix="file-trace" />}
    </summary>
    <div className="flex min-w-0 flex-col gap-1 border-t border-border p-2">
      {group.calls.map((call, index) => <TimelineToolCallCard key={`${index}:${call.callId}`} conversationId={conversationId} toolCall={call} parentRunActive={parentRunActive} trace />)}
    </div>
  </details>;
});
