import { memo, useMemo } from 'react';
import { AlertCircle, ChevronRight, Loader2, Monitor } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { useStreamTool } from '../../lib/useStreamSelector';
import type { TimelineSection } from '../../lib/streaming/timelineViewModel';
import { isPendingToolCallStatus, isUnsuccessfulToolCallStatus } from '../../lib/streaming/toolStatus';
import { TimelineToolCallCard } from './ToolCallCard';

export type ComputerUseStep = Extract<TimelineSection, { kind: 'tool' }>;

export function isComputerUseStep(section: TimelineSection): section is ComputerUseStep {
  return section.kind === 'tool' && ['computer_observe', 'computer_control', 'desktop_automation'].includes(section.toolCall.toolName ?? '');
}

/** One visible workflow, retaining the original per-action receipts and the
 * read-only/input permission split. No new dispatcher or tool aliases. */
export const ComputerUseTrace = memo(function ComputerUseTrace({ steps, conversationId, parentRunActive }: {
  steps: ComputerUseStep[];
  conversationId?: string | null;
  parentRunActive: boolean;
}) {
  const { t } = useTranslation();
  const last = steps[steps.length - 1].toolCall;
  const live = useStreamTool(parentRunActive ? conversationId ?? '' : '', last.callId, last);
  const current = live ?? last;
  const pending = parentRunActive && isPendingToolCallStatus(current.status);
  const failed = isUnsuccessfulToolCallStatus(current.status) || current.isError;
  const previousFailures = useMemo(() => steps.slice(0, -1).some(step => isUnsuccessfulToolCallStatus(step.toolCall.status) || step.toolCall.isError), [steps]);
  const Icon = pending ? Loader2 : failed || previousFailures ? AlertCircle : Monitor;
  return <details className="group/computer-use my-1 max-w-full rounded-lg border border-border bg-surface-1/50" data-testid="computer-use-trace">
    <summary className="flex min-h-10 cursor-pointer list-none items-center gap-2 px-3 py-2 text-xs text-text-secondary" aria-busy={pending}>
      <ChevronRight size={14} className="shrink-0 transition-transform group-open/computer-use:rotate-90" />
      <Icon size={14} className={`${pending ? 'animate-spin motion-reduce:animate-none' : ''} ${failed || previousFailures ? 'text-amber-500' : 'text-accent'}`} />
      <span className="font-medium text-text-primary">Computer Use</span>
      <span>{t('chat.workflowTasks', { count: String(steps.length) })}</span>
      {pending && <span className="min-w-0 truncate">{t(current.toolName === 'computer_observe' ? 'chat.computerUseObserving' : 'chat.computerUseOperating')}</span>}
      {(failed || previousFailures) && <span>{t('chat.toolBriefError')}</span>}
    </summary>
    <div className="flex min-w-0 flex-col gap-1 border-t border-border p-2">
      {steps.map(step => <TimelineToolCallCard key={step.id} conversationId={conversationId} toolCall={step.toolCall} parentRunActive={parentRunActive} trace={step.trace} />)}
    </div>
  </details>;
});
