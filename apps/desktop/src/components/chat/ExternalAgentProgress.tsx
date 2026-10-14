import { useEffect, useState } from 'react';
import { AlertTriangle, CheckCircle2, Loader2 } from 'lucide-react';
import { useTranslation } from '../../i18n';
import type { TraceEvent } from '../../lib/streaming/protocol';
import { externalAgentProgress } from '../../lib/externalAgentProgress';

const labels = {
  starting: 'chat.agentStarting', connecting: 'chat.agentConnecting',
  model: 'chat.agentPreparing', reusing: 'chat.agentReusing', waiting: 'chat.agentWaiting',
  retrying: 'chat.agentWaiting',
} as const;

export function ExternalAgentProgress({ events, active }: { events: TraceEvent[]; active: boolean }) {
  const { t } = useTranslation();
  const { latest, stage, compacting } = externalAgentProgress(events, active);
  const receipt = stage === 'command_completed';
  const warning = stage === 'warning';
  const visible = compacting || (stage !== undefined && ((active && stage in labels) || receipt || warning));
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    setElapsed(0);
    if (!visible || receipt || warning) return;
    const started = Date.now();
    const timer = window.setInterval(() => setElapsed(Math.floor((Date.now() - started) / 1000)), 1000);
    return () => window.clearInterval(timer);
  }, [visible, receipt, warning, stage, compacting]);
  if (!visible) return null;
  return (
    <div role="status" data-testid="external-agent-progress" className="mx-auto flex max-w-4xl items-center gap-2 px-6 pb-2 text-xs text-text-secondary">
      {receipt ? <CheckCircle2 size={13} className="shrink-0 text-success" /> : warning ? <AlertTriangle size={13} className="shrink-0 text-warning" /> : <Loader2 size={13} className="shrink-0 animate-spin motion-reduce:animate-none" />}
      <span className="min-w-0 break-words">{compacting ? t('chat.agentCompactingBackground') : receipt || warning ? latest?.text : t(labels[stage as keyof typeof labels])}</span>
      {!receipt && !warning && elapsed >= 3 && <span aria-hidden="true" className="ml-auto tabular-nums opacity-65">{elapsed}s</span>}
    </div>
  );
}
