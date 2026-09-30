import { useEffect, useState } from 'react';
import { Loader2 } from 'lucide-react';
import { useTranslation } from '../../i18n';
import type { TraceEvent } from '../../lib/streaming/protocol';

const labels = {
  starting: 'chat.agentStarting', connecting: 'chat.agentConnecting',
  model: 'chat.agentPreparing', reusing: 'chat.agentReusing', waiting: 'chat.agentWaiting',
  compacting: 'chat.compacting',
} as const;

export function ExternalAgentProgress({ events, active }: { events: TraceEvent[]; active: boolean }) {
  const { t } = useTranslation();
  const latest = [...events].reverse().find(event => event.kind === 'status' && event.code?.startsWith('external_agent_'));
  const stage = latest?.kind === 'status' ? latest.code?.replace('external_agent_', '') : undefined;
  const visible = active && stage !== undefined && stage in labels;
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    setElapsed(0);
    if (!visible) return;
    const started = Date.now();
    const timer = window.setInterval(() => setElapsed(Math.floor((Date.now() - started) / 1000)), 1000);
    return () => window.clearInterval(timer);
  }, [visible]);
  if (!visible) return null;
  return (
    <div role="status" data-testid="external-agent-progress" className="mx-auto flex max-w-4xl items-center gap-2 px-6 pb-2 text-xs text-text-secondary">
      <Loader2 size={13} className="shrink-0 animate-spin motion-reduce:animate-none" />
      <span>{t(labels[stage as keyof typeof labels])}</span>
      {elapsed >= 3 && <span aria-hidden="true" className="ml-auto tabular-nums opacity-65">{elapsed}s</span>}
    </div>
  );
}
