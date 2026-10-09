import { memo, useMemo } from 'react';
import type {
  AgentTaskRun,
  AgentTaskRunEvent,
  ConversationMessage,
} from '../../types/conversation';
import type { ToolCallEvent } from '../../lib/streaming/protocol';
import type { ActiveGoalContext } from '../../lib/goalContext';
import {
  extractPlanArtifact,
  findLatestPlanArtifact,
  findLatestSubtaskArtifacts,
  type SubtaskRunArtifact,
} from '../../lib/taskArtifacts';
import { PlanProgressPanel } from './TaskPanels';
import { useGitWorkspace } from '../../lib/gitWorkspace';

interface TaskBoardProps {
  conversationId?: string | null;
  isStreaming?: boolean;
  sourceRevision?: string;
  messages: ConversationMessage[];
  toolCalls: ToolCallEvent[];
  taskRun?: AgentTaskRun | null;
  taskEvents?: AgentTaskRunEvent[];
  goal?: ActiveGoalContext | null;
  onOpenSubagent?: (subtask: SubtaskRunArtifact) => void;
  subagentStatuses?: Record<string, string>;
}

export const TaskBoard = memo(function TaskBoard({
  conversationId,
  isStreaming = false,
  sourceRevision = '',
  messages,
  toolCalls,
  taskRun,
  taskEvents = [],
  goal = null,
  onOpenSubagent,
  subagentStatuses,
}: TaskBoardProps) {
  const git = useGitWorkspace(conversationId, isStreaming, `${sourceRevision}:${messages.length}:${toolCalls.filter(call => call.status !== 'running').length}`);
  const plan = useMemo(
    () => findLatestUpdatePlanArtifact(messages, toolCalls)
      ?? findLatestPlanArtifact(messages, toolCalls, taskRun?.plan),
    [messages, taskRun?.plan, toolCalls],
  );
  const subtasks = useMemo(
    () => findLatestSubtaskArtifacts(
      taskRun?.userMessageId && messages.some(message => message.id === taskRun.userMessageId)
        ? messages.slice(messages.findIndex(message => message.id === taskRun.userMessageId))
        : messages,
      toolCalls,
      taskRun?.artifacts,
      taskEvents,
      isStreaming,
    ),
    [messages, taskEvents, taskRun?.artifacts, taskRun?.userMessageId, toolCalls, isStreaming],
  );

  const hasGitContext = git.checkedSources > 0 || git.issues.length > 0 || !!git.error;
  if (!plan && !goal && subtasks.length === 0 && !hasGitContext) {
    return null;
  }

  if (!goal && plan?.routeKind === 'DirectResponse' && subtasks.length === 0 && !hasGitContext) {
    return null;
  }

  return (
    <div
      data-testid="task-board"
      className="pointer-events-none absolute right-3 top-14 z-20 w-[min(22rem,calc(100%-1.5rem))] md:right-4"
    >
      <PlanProgressPanel key={conversationId} plan={plan?.routeKind === 'DirectResponse' ? null : plan} goal={goal} subtasks={subtasks.map(task => {
        const live = subagentStatuses?.[task.rowId || task.lifecycleId || task.id];
        if (!live) return task;
        const active = ['queued', 'running', 'cancelling'].includes(live);
        return { ...task, status: live === 'cancelling' ? 'running' : live === 'orphaned' ? 'failed' : live, runtimeState: active ? 'live' : 'terminal' };
      })} git={git} conversationId={conversationId} onOpenSubagent={onOpenSubagent} />
    </div>
  );
});

function isUpdatePlanTool(toolName: string | null | undefined) {
  return toolName?.trim().toLowerCase() === 'update_plan';
}

function findLatestUpdatePlanArtifact(
  messages: ConversationMessage[],
  toolCalls: ToolCallEvent[],
) {
  for (let i = toolCalls.length - 1; i >= 0; i -= 1) {
    const call = toolCalls[i];
    if (!isUpdatePlanTool(call.toolName)) continue;
    const artifact = extractPlanArtifact(call.artifacts);
    if (artifact) return artifact;
  }

  const updatePlanCallIds = new Set<string>();
  for (const message of messages) {
    for (const call of message.toolCalls) {
      if (isUpdatePlanTool(call.name)) {
        updatePlanCallIds.add(call.id);
      }
    }
  }

  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i];
    const hasUpdatePlanCall =
      message.toolCalls.some(call => isUpdatePlanTool(call.name))
      || (message.toolCallId ? updatePlanCallIds.has(message.toolCallId) : false);

    if (!hasUpdatePlanCall) continue;
    const artifact = extractPlanArtifact(message.artifacts);
    if (artifact) return artifact;
  }

  return null;
}
