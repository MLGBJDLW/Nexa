import { useEffect, useState } from 'react';
import { AlertTriangle, CheckCircle2, Info, Loader2 } from 'lucide-react';
import { useTranslation } from '../../i18n';
import type { TraceEvent } from '../../lib/streaming/protocol';
import { externalAgentProgress } from '../../lib/externalAgentProgress';

const labels = {
  starting: 'chat.agentStarting', connecting: 'chat.agentConnecting',
  model: 'chat.agentPreparing', reusing: 'chat.agentReusing', waiting: 'chat.agentWaiting',
  retrying: 'chat.agentWaiting',
  receiving: 'chat.agentReceiving',
} as const;

export function ExternalAgentProgress({ events, active }: { events: TraceEvent[]; active: boolean }) {
  const { t } = useTranslation();
  const { latest, stage, compacting, reasoningUnavailable } = externalAgentProgress(events, active);
  const receipt = stage === 'command_completed';
  const warning = stage === 'warning';
  const activity = stage === 'activity';
  const visible = compacting || (stage !== undefined && ((active && (stage in labels || activity)) || receipt || warning));
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    setElapsed(0);
    if (!visible || receipt || warning) return;
    const started = Date.now();
    const timer = window.setInterval(() => setElapsed(Math.floor((Date.now() - started) / 1000)), 1000);
    return () => window.clearInterval(timer);
  }, [visible, receipt, warning, stage, compacting]);
  if (!visible && !reasoningUnavailable) return null;
  return (
    <div role="status" data-testid="external-agent-progress" className="mx-auto flex max-w-4xl flex-wrap items-center gap-2 px-6 pb-2 text-xs text-text-secondary">
      {visible && <>{receipt ? <CheckCircle2 size={13} className="shrink-0 text-success" /> : warning ? <AlertTriangle size={13} className="shrink-0 text-warning" /> : <Loader2 size={13} className="shrink-0 animate-spin motion-reduce:animate-none" />}
      <span className="min-w-0 break-words">{compacting ? t('chat.agentCompactingBackground') : receipt || warning || activity ? latest?.text : t(labels[stage as keyof typeof labels])}</span>
      {!receipt && !warning && elapsed >= 3 && <span aria-hidden="true" className="ml-auto tabular-nums opacity-65">{elapsed}s</span>}</>}
      {reasoningUnavailable && <span className="flex min-w-0 items-center gap-1.5 text-text-tertiary" data-testid="external-agent-reasoning-unavailable"><Info size={12} className="shrink-0" />{t('chat.agentReasoningUnavailable')}</span>}
    </div>
  );
}
