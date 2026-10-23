import type { TraceEvent, TraceStatusEvent } from './streaming/protocol';

/** Native compaction is independent of foreground generation/tool progress. */
export function externalAgentProgress(events: TraceEvent[], active: boolean) {
  let latest: TraceStatusEvent | undefined;
  let compaction: TraceStatusEvent | undefined;
  let reasoningUnavailable = false;
  for (const event of events) {
    if (event.kind !== 'status' || !event.code?.startsWith('external_agent_')) continue;
    if (event.code === 'external_agent_compacting' || event.code === 'external_agent_compaction_complete') compaction = event;
    else if (event.code === 'external_agent_reasoning_unavailable') reasoningUnavailable = true;
    else if (event.code === 'external_agent_network_progress') {
      if (!latest || ['starting', 'connecting', 'model', 'reusing', 'waiting', 'retrying'].includes(latest.code?.slice('external_agent_'.length) ?? '')) {
        latest = { ...event, code: 'external_agent_receiving' };
      }
    } else latest = event;
  }
  return { latest, stage: latest?.code?.slice('external_agent_'.length),
    reasoningUnavailable: reasoningUnavailable && !events.some(event => event.kind === 'thinking' && event.text.trim()),
    compacting: active && compaction?.code === 'external_agent_compacting' };
}
