import { memo, useCallback } from 'react';
import { AlertCircle, CheckCircle2, ChevronRight, Loader2, Monitor } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { useStreamSelector } from '../../lib/useStreamSelector';
import { streamStore } from '../../lib/streamStore';
import type { TimelineSection } from '../../lib/streaming/timelineViewModel';
import { interactionPresentation, isInteractionTool, summarizeInteractions } from '../../lib/streaming/interactionPresentation';
import { TimelineToolCallCard } from './ToolCallCard';

export type InteractionStep = Extract<TimelineSection, { kind: 'tool' }>;

export function isInteractionStep(section: TimelineSection): section is InteractionStep {
  return section.kind === 'tool' && isInteractionTool(section.toolCall.toolName ?? '');
}

/** One visible workflow, retaining the original per-action receipts and the
 * read-only/input permission split. No new dispatcher or tool aliases. */
export const InteractionTrace = memo(function InteractionTrace({ steps, conversationId, parentRunActive }: {
  steps: InteractionStep[];
  conversationId?: string | null;
  parentRunActive: boolean;
}) {
  const { t } = useTranslation();
  const current = useStreamSelector(parentRunActive ? conversationId ?? '' : '', useCallback(() => summarizeInteractions(steps.map(step => {
    // Occurrence identity prevents a reused provider call ID from updating a
    // historical step. Every active step participates, not just the last one.
    const call = parentRunActive ? streamStore.selectTool(conversationId ?? '', step.toolCall.callId, step.toolCall) ?? step.toolCall : step.toolCall;
    return interactionPresentation(call, parentRunActive)!;
  })), [conversationId, parentRunActive, steps]));
  const Icon = current.pending ? Loader2 : current.tone === 'danger' || current.tone === 'warning' ? AlertCircle : current.tone === 'success' ? CheckCircle2 : Monitor;
  const tone = current.tone === 'danger' ? 'text-danger' : current.tone === 'warning' ? 'text-warning' : current.tone === 'success' ? 'text-success' : 'text-text-tertiary';
  return <details className="group/interaction my-1 max-w-full rounded-lg border border-border bg-surface-1/50" data-testid="interaction-trace">
    <summary className="flex min-h-10 cursor-pointer list-none items-center gap-2 px-3 py-2 text-xs text-text-secondary" aria-busy={current.pending}>
      <ChevronRight size={14} className="shrink-0 transition-transform group-open/interaction:rotate-90" />
      <Icon size={14} className={`shrink-0 ${current.pending ? 'animate-spin motion-reduce:animate-none' : ''} ${tone}`} />
      <span className="shrink-0 font-medium text-text-primary">{t('chat.interactionTitle')}</span>
      <span>{t('chat.workflowTasks', { count: String(steps.length) })}</span>
      <span className={`min-w-0 truncate ${tone}`}>{t(current.labelKey)}</span>
    </summary>
    <div className="flex min-w-0 flex-col gap-1 border-t border-border p-2">
      {steps.map(step => <TimelineToolCallCard key={step.id} conversationId={conversationId} toolCall={step.toolCall} parentRunActive={parentRunActive} trace={step.trace} />)}
    </div>
  </details>;
});
