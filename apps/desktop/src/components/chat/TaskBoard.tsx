import { ChevronDown, ClipboardList } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useTranslation } from '../../i18n';
import type { ConversationMessage } from '../../types/conversation';
import type { ToolCallEvent } from '../../lib/useAgentStream';
import {
  findLatestPlanArtifact,
} from '../../lib/taskArtifacts';
import { PlanPanel } from './TaskPanels';

interface TaskBoardProps {
  messages: ConversationMessage[];
  toolCalls: ToolCallEvent[];
}

export function TaskBoard({
  messages,
  toolCalls,
}: TaskBoardProps) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(true);
  const plan = useMemo(
    () => findLatestPlanArtifact(messages, toolCalls),
    [messages, toolCalls],
  );

  if (!plan) {
    return null;
  }

  const completed = plan?.steps.filter(step => step.status === 'completed').length ?? 0;
  const total = plan?.steps.length ?? 0;

  return (
    <div
      data-testid="task-board"
      className="shrink-0 border-t border-border/60 bg-surface-0/90 px-3 py-2 backdrop-blur"
    >
      <details
        open={expanded}
        onToggle={(event) => setExpanded(event.currentTarget.open)}
        className="group rounded-lg border border-border/60 bg-surface-1/55"
      >
        <summary className="flex cursor-pointer list-none items-center gap-1.5 px-2 py-1.5 text-xs text-text-secondary [&::-webkit-details-marker]:hidden">
          <span className="inline-flex min-w-0 items-center gap-1.5 rounded-full border border-border/60 bg-surface-1/70 px-2 py-1 text-[11px] text-text-primary">
            <ClipboardList className="h-3 w-3 text-accent" />
            <span className="truncate">{t('chat.taskBoard')}</span>
            <span className="tabular-nums text-text-tertiary">{completed}/{total}</span>
          </span>

          <span className="ml-auto inline-flex items-center gap-1 text-[11px] text-text-tertiary transition-colors group-hover:text-text-secondary">
            {t('chat.taskBoardDetails')}
            <ChevronDown className="h-3.5 w-3.5 shrink-0 transition-transform group-open:rotate-180" />
          </span>
        </summary>

        <div className="max-h-[200px] overflow-y-auto border-t border-border/60 px-2 pb-2 pt-1.5">
          <PlanPanel plan={plan} compact />
        </div>
      </details>
    </div>
  );
}
