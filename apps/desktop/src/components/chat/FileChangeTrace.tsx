import { memo } from 'react';
import type { ToolCallEvent } from '../../lib/streaming/protocol';
import { isPendingToolCallStatus, isUnsuccessfulToolCallStatus } from '../../lib/streaming/toolStatus';
import { useStreamSelector } from '../../lib/useStreamSelector';
import { streamStore } from '../../lib/streamStore';
import { diffPathAliasKeys, extractDiffStatsArtifact, extractFileDiffArtifacts } from './FileDiffPreview';
import { TimelineToolCallCard, ToolCallCard } from './ToolCallCard';

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
  const liveId = parentRunActive ? conversationId ?? '' : '';
  // Subscribe to receipt updates as well as newly appended calls. Occurrence
  // lookup prevents a later run reusing a call ID from replacing history.
  useStreamSelector(liveId, state => state?.toolCalls);
  const calls = group.calls.map(call => streamStore.selectTool(liveId, call.callId, call) ?? call);
  const pending = parentRunActive ? calls.find(call => isPendingToolCallStatus(call.status)) : undefined;
  const failed = calls.find(call => call.isError || isUnsuccessfulToolCallStatus(call.status));
  const representative = pending ?? failed ?? calls[calls.length - 1];
  const accumulated = calls.length > 1;
  // These are cumulative operation counts; the existing file-changes dock
  // remains the authority for the net on-disk diff and baseline readback.
  // A failed edit can still carry a confirmed partial-write receipt. Count
  // those real changes, never the failed call's speculative arguments.
  const stats = calls.map(call => extractDiffStatsArtifact(call.artifacts));
  const additions = stats.reduce((total, item) => total + (item?.additions ?? 0), 0);
  const deletions = stats.reduce((total, item) => total + (item?.deletions ?? 0), 0);
  return <div className="min-w-0 max-w-full" data-testid="file-change-trace" data-file-path={group.path}>
    <ToolCallCard
      {...representative}
      toolName={accumulated ? 'edit_file' : representative.toolName}
      status={failed && !pending ? 'error' : representative.status}
      artifacts={accumulated ? { diffStats: {
        kind: 'diffStats', operation: 'edit_file', filesChanged: 1,
        additions, deletions, hunks: stats.reduce((total, item) => total + (item?.hunks ?? 0), 0),
        paths: [group.path],
      } } : representative.artifacts}
      parentRunActive={parentRunActive}
      trace
      detailContent={accumulated ? calls.map((call, index) => (
        <TimelineToolCallCard key={`${index}:${call.callId}`} conversationId={conversationId} toolCall={call} parentRunActive={parentRunActive} trace />
      )) : undefined}
    />
  </div>;
});
