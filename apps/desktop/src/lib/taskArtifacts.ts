import type {
  AgentTaskRunEvent,
  ActivityEvent,
  ConversationMessage,
} from '../types/conversation';
import type { ToolCallEvent } from './streaming/protocol';
import { taskTimelinePayloadFromTaskEvent } from './streaming/taskTimeline';

export type PlanStepStatus = 'pending' | 'in_progress' | 'completed';
export type VerificationStatus = 'pending' | 'passed' | 'failed' | 'skipped';
export type VerificationOverallStatus = 'pending' | 'passed' | 'failed' | 'partial';

export interface PlanStepArtifact {
  id?: string | null;
  title: string;
  status: PlanStepStatus;
  notes?: string | null;
}

export interface PlanArtifact {
  kind: 'plan';
  title?: string | null;
  explanation?: string | null;
  routeKind?: string | null;
  steps: PlanStepArtifact[];
  counts?: {
    total?: number;
    completed?: number;
    inProgress?: number;
    pending?: number;
  } | null;
  updatedAt?: string | null;
}

export interface VerificationCheckArtifact {
  name: string;
  status: VerificationStatus;
  details?: string | null;
}

export interface RuntimeVerificationGateArtifact {
  required: boolean;
  reasons: string[];
  verificationArtifactStatus?: VerificationOverallStatus | null;
}

export interface VerificationArtifact {
  kind: 'verification';
  summary?: string | null;
  overallStatus?: VerificationOverallStatus | null;
  checks: VerificationCheckArtifact[];
  counts?: {
    total?: number;
    passed?: number;
    failed?: number;
    pending?: number;
    skipped?: number;
  } | null;
  runtimeGate?: RuntimeVerificationGateArtifact | null;
  updatedAt?: string | null;
}

export interface SubtaskRunArtifact {
  id: string;
  label: string;
  role?: string | null;
  status: string;
  phase?: string | null;
  task?: string | null;
  result?: string | null;
  errorMessage?: string | null;
  tokenBudget?: number | null;
  identityAliases?: string[];
  lifecycleId?: string | null;
  rowId?: string | null;
}

export function compactTaskLabel(value: string, maxCharacters = 72): string {
  const firstLine = value.trim().split(/\r?\n/).find(line => line.trim()) ?? '';
  const normalized = firstLine.replace(/^\s*(?:#{1,6}|[-*])\s+/, '').replace(/\s+/g, ' ').trim();
  const sentence = normalized.match(/^(.{12,}?[。！？.!?])(?:\s|$)/u)?.[1] ?? normalized;
  const points = Array.from(sentence);
  return points.length <= maxCharacters ? sentence : `${points.slice(0, maxCharacters - 1).join('')}…`;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

function asText(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function asNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function normalizePlanArtifact(value: unknown): PlanArtifact | null {
  const record = asRecord(value);
  if (!record || !Array.isArray(record.steps)) return null;
  const isClassicPlan = record.kind === 'plan';
  const isTypedTaskPlan = typeof record.routeKind === 'string' && typeof record.version === 'number';
  if (!isClassicPlan && !isTypedTaskPlan) return null;

  const steps: PlanStepArtifact[] = record.steps
    .map((step): PlanStepArtifact | null => {
      const item = asRecord(step);
      if (!item || typeof item.title !== 'string' || typeof item.status !== 'string') return null;
      const status = normalizePlanStepStatus(item.status);
      if (!['pending', 'in_progress', 'completed'].includes(status)) return null;
      return {
        id: typeof item.id === 'string' ? item.id : null,
        title: item.title.trim(),
        status,
        notes: typeof item.notes === 'string' ? item.notes : null,
      };
    })
    .filter((step): step is PlanStepArtifact => Boolean(step && step.title));

  if (steps.length === 0) return null;

  const counts = asRecord(record.counts);
  return {
    kind: 'plan',
    title: typeof record.title === 'string' ? record.title : null,
    explanation: typeof record.explanation === 'string' ? record.explanation : null,
    routeKind: typeof record.routeKind === 'string' ? record.routeKind : null,
    steps,
    counts: counts
      ? {
          total: typeof counts.total === 'number' ? counts.total : undefined,
          completed: typeof counts.completed === 'number' ? counts.completed : undefined,
          inProgress: typeof counts.inProgress === 'number' ? counts.inProgress : undefined,
          pending: typeof counts.pending === 'number' ? counts.pending : undefined,
        }
      : null,
    updatedAt: typeof record.updatedAt === 'string' ? record.updatedAt : null,
  };
}

function normalizePlanStepStatus(value: string): PlanStepStatus {
  if (value === 'inProgress') return 'in_progress';
  if (value === 'completed' || value === 'pending' || value === 'in_progress') {
    return value;
  }
  return 'pending';
}

function normalizeVerificationArtifact(value: unknown): VerificationArtifact | null {
  const record = asRecord(value);
  if (!record || record.kind !== 'verification' || !Array.isArray(record.checks)) return null;

  const checks: VerificationCheckArtifact[] = record.checks
    .map((check): VerificationCheckArtifact | null => {
      const item = asRecord(check);
      if (!item || typeof item.name !== 'string' || typeof item.status !== 'string') return null;
      const status = item.status as VerificationStatus;
      if (!['pending', 'passed', 'failed', 'skipped'].includes(status)) return null;
      return {
        name: item.name.trim(),
        status,
        details: typeof item.details === 'string' ? item.details : null,
      };
    })
    .filter((check): check is VerificationCheckArtifact => Boolean(check && check.name));

  if (checks.length === 0) return null;

  const overall = record.overallStatus;
  const counts = asRecord(record.counts);
  const runtimeGate = normalizeRuntimeGate(record.runtimeGate);
  return {
    kind: 'verification',
    summary: typeof record.summary === 'string' ? record.summary : null,
    overallStatus:
      typeof overall === 'string' &&
      ['pending', 'passed', 'failed', 'partial'].includes(overall)
        ? (overall as VerificationOverallStatus)
        : null,
    checks,
    counts: counts
      ? {
          total: typeof counts.total === 'number' ? counts.total : undefined,
          passed: typeof counts.passed === 'number' ? counts.passed : undefined,
          failed: typeof counts.failed === 'number' ? counts.failed : undefined,
          pending: typeof counts.pending === 'number' ? counts.pending : undefined,
          skipped: typeof counts.skipped === 'number' ? counts.skipped : undefined,
        }
      : null,
    runtimeGate,
    updatedAt: typeof record.updatedAt === 'string' ? record.updatedAt : null,
  };
}

function normalizeRuntimeGate(value: unknown): RuntimeVerificationGateArtifact | null {
  const record = asRecord(value);
  if (!record) return null;
  const verificationArtifactStatus = record.verificationArtifactStatus;
  return {
    required: record.required === true,
    reasons: Array.isArray(record.reasons)
      ? record.reasons.filter((reason): reason is string => typeof reason === 'string')
      : [],
    verificationArtifactStatus:
      typeof verificationArtifactStatus === 'string'
        && ['pending', 'passed', 'failed', 'partial'].includes(verificationArtifactStatus)
        ? (verificationArtifactStatus as VerificationOverallStatus)
        : null,
  };
}

function subtaskStatus(status: string): string {
  if (status === 'done') return 'completed';
  if (status === 'error' || status === 'timed_out' || status === 'timedOut') return 'failed';
  if (['connecting', 'first_token', 'thinking', 'tool_running', 'cancelling'].includes(status)) return 'running';
  return status;
}

function normalizeSubtaskRun(
  value: unknown,
  trustedSubtaskContainer = false,
): SubtaskRunArtifact | null {
  const record = asRecord(value);
  if (!record) return null;

  const input = asRecord(record.input);
  const output = asRecord(record.output);
  const outputRun = asRecord(output?.run);
  const outputJudgement = asRecord(output?.judgement);
  const kind = asText(record.kind)?.toLowerCase();
  const inputKind = asText(input?.kind)?.toLowerCase();
  const hasSubagentProvenance = Boolean(
    trustedSubtaskContainer
    || kind?.startsWith('subagent_')
    || kind === 'subtask'
    || inputKind?.startsWith('subagent_')
    || (asText(record.parentRunId) && asText(record.role)),
  );
  if (!hasSubagentProvenance) return null;

  const directRun = trustedSubtaskContainer || kind?.startsWith('subagent_') ? record : null;
  const run = outputRun ?? outputJudgement ?? directRun;

  const lifecycleId = asText(record.agentId) ?? asText(input?.agentId);
  const id = lifecycleId ??
    asText(input?.callLabel) ??
    asText(record.id) ??
    asText(input?.callLabel) ??
    asText(run?.id) ??
    asText(run?.task) ??
    asText(record.label);
  const task = asText(input?.task) ?? asText(run?.task);
  const storedLabel = asText(record.label);
  const label =
    (storedLabel !== id && storedLabel !== asText(input?.callLabel) ? storedLabel : null) ??
    asText(input?.task) ??
    asText(run?.task) ??
    asText(run?.summary) ??
    id;
  if (!id || !label) return null;

  const rawStatus =
    asText(record.status) ??
    asText(run?.status) ??
    (record.isError === true || run?.isError === true ? 'failed' : null) ??
    'completed';
  const status = subtaskStatus(rawStatus);
  const role =
    asText(record.role) ??
    asText(input?.roleName) ??
    asText(input?.role) ??
    asText(run?.roleName) ??
    asText(run?.role);

  return {
    id,
    label: compactTaskLabel(label),
    role,
    status,
    phase: asText(record.phase),
    task,
    result: asText(run?.result) ?? asText(run?.summary),
    errorMessage: asText(record.errorMessage) ?? asText(run?.errorMessage) ?? asText(output?.error),
    tokenBudget: asNumber(record.tokenBudget) ?? asNumber(input?.reservedTokens),
    lifecycleId,
    rowId: asText(record.parentRunId) ? asText(record.id) : null,
    identityAliases: [...new Set([id, asText(record.id), asText(input?.callLabel), asText(run?.id), asText(record.workerId)].filter((value): value is string => Boolean(value)))],
  };
}

function normalizeSubtaskArtifacts(
  value: unknown,
  trustedSubtaskContainer = false,
): SubtaskRunArtifact[] | null {
  if (Array.isArray(value)) {
    const subtasks = value
      .map(item => normalizeSubtaskRun(item, trustedSubtaskContainer))
      .filter((subtask): subtask is SubtaskRunArtifact => Boolean(subtask));
    return trustedSubtaskContainer ? subtasks : (subtasks.length ? subtasks : null);
  }

  const record = asRecord(value);
  if (!record) return null;

  const kind = asText(record.kind)?.toLowerCase();
  // observe/wait/close return an authoritative worker snapshot, not another
  // spawn artifact. A close only succeeds for a terminal worker.
  if (kind === 'subagent_input_queued') return [];
  const worker = asRecord(record.worker)
    ?? (kind === 'subagent_observation' ? asRecord(asRecord(record.observation)?.worker) : null);
  if (kind?.startsWith('subagent_') && worker) {
    const result = asRecord(worker.result);
    const subtask = normalizeSubtaskRun({
      ...worker, id: worker.agentId, workerId: result?.id,
      result: result?.result, errorMessage: worker.errorMessage,
    }, true);
    return subtask ? [subtask] : null;
  }
  if (kind === 'subagent_cancellation') {
    const subtask = normalizeSubtaskRun({ ...record, id: record.agentId, label: record.agentId }, true);
    return subtask ? [subtask] : null;
  }
  if (kind === 'subagent_batch_result' && Array.isArray(record.runs)) {
    const workers = Array.isArray(record.lifecycleWorkers) ? record.lifecycleWorkers.map(asRecord) : [];
    return record.runs.map(run => {
      const item = asRecord(run);
      const worker = workers.find(worker => worker && (worker.workerId === item?.id || worker.agentId === item?.id));
      return normalizeSubtaskRun({ ...item, agentId: worker?.agentId }, true);
    }).filter((run): run is SubtaskRunArtifact => Boolean(run));
  }

  if (Array.isArray(record.subtasks)) {
    // `subtasks` is the canonical backend projection. An explicit empty array
    // is authoritative and must stop recursive artifact discovery from
    // misclassifying sibling skills, workflow runs, or ordinary tool records.
    return normalizeSubtaskArtifacts(record.subtasks, true);
  }
  if (
    Array.isArray(record.runs)
    && ['subagents', 'subagent_runs', 'subtask_runs'].includes(asText(record.kind)?.toLowerCase() ?? '')
  ) {
    return normalizeSubtaskArtifacts(record.runs, true);
  }
  const single = normalizeSubtaskRun(record);
  return single ? [single] : null;
}

function mergeSubtaskArtifacts(
  target: Map<string, SubtaskRunArtifact>,
  subtasks: SubtaskRunArtifact[],
) {
  for (const subtask of subtasks) {
    const key = subtask.id || subtask.label;
    const aliases = new Set(subtask.identityAliases ?? [key]);
    const matches = [...target.entries()].filter(([, previous]) => {
      if (previous.lifecycleId && subtask.lifecycleId && previous.lifecycleId !== subtask.lifecycleId) return false;
      if (previous.rowId && subtask.rowId && previous.rowId !== subtask.rowId) return false;
      return (previous.identityAliases ?? [previous.id]).some(alias => aliases.has(alias));
    });
    const previousEntry = target.has(key) ? [key, target.get(key)!] as const : matches.length === 1 ? matches[0] : undefined;
    const previous = previousEntry?.[1];
    if (previousEntry && previousEntry[0] !== key) target.delete(previousEntry[0]);
    const terminal = (status: string) => ['completed', 'failed', 'cancelled'].includes(status);
    const keepTerminal = previous && terminal(previous.status) && !terminal(subtask.status);
    target.set(key, previous
      ? {
          ...previous,
          ...subtask,
          role: subtask.role ?? previous.role,
          task: subtask.task ?? previous.task,
          result: subtask.result ?? previous.result,
          errorMessage: subtask.errorMessage ?? previous.errorMessage,
          tokenBudget: subtask.tokenBudget ?? previous.tokenBudget,
          status: keepTerminal ? previous.status : subtask.status,
          phase: keepTerminal ? previous.phase : subtask.phase ?? previous.phase,
          label: subtask.label === key ? previous.label : subtask.label,
          identityAliases: [...new Set([...(previous.identityAliases ?? [previous.id]), ...aliases])],
          lifecycleId: subtask.lifecycleId ?? previous.lifecycleId,
          rowId: subtask.rowId ?? previous.rowId,
        }
      : subtask);
  }
}

function subtaskArtifactFromTimelineEvent(
  event: AgentTaskRunEvent,
): SubtaskRunArtifact | null {
  const timeline = taskTimelinePayloadFromTaskEvent(event);
  if (!timeline || timeline.kind !== 'subtask') return null;

  const payload = asRecord(timeline.payload);
  const nestedRun = asRecord(payload?.run) ?? asRecord(payload?.judgement);
  const callLabel = asText(payload?.callLabel) ?? asText(nestedRun?.id);
  const id = callLabel ?? asText(payload?.subtaskRunId) ?? event.id;
  const task = asText(payload?.task) ?? asText(nestedRun?.task);
  const rawStatus = asText(timeline.status) ?? asText(event.status) ?? 'queued';
  // Usage and judgement telemetry describes an existing worker, not its state.
  if (['telemetry', 'judging', 'judged'].includes(rawStatus)) return null;
  const status = subtaskStatus(rawStatus);

  return {
    id,
    label: compactTaskLabel(task ?? callLabel ?? timeline.label ?? event.label),
    role: asText(payload?.role) ?? asText(nestedRun?.roleName) ?? asText(nestedRun?.role),
    status,
    phase: asText(payload?.phase),
    task,
    result: asText(payload?.result) ?? asText(nestedRun?.result) ?? asText(nestedRun?.summary),
    errorMessage: asText(payload?.error) ?? asText(nestedRun?.errorMessage),
    tokenBudget: asNumber(payload?.reservedTokens) ?? asNumber(payload?.tokenBudget),
    identityAliases: [id, asText(payload?.subtaskRunId)].filter((value): value is string => Boolean(value)),
  };
}

function extractNestedArtifact<T>(
  value: unknown,
  normalize: (candidate: unknown) => T | null,
  depth = 0,
): T | null {
  const direct = normalize(value);
  if (direct) return direct;
  if (depth >= 6 || value == null) return null;

  if (Array.isArray(value)) {
    for (let i = value.length - 1; i >= 0; i -= 1) {
      const found = extractNestedArtifact(value[i], normalize, depth + 1);
      if (found) return found;
    }
    return null;
  }

  const record = asRecord(value);
  if (!record) return null;

  const preferredKeys = ['artifacts', 'toolCall', 'toolCalls', 'items'];
  const visited = new Set<string>();
  for (const key of preferredKeys) {
    if (!(key in record)) continue;
    visited.add(key);
    const found = extractNestedArtifact(record[key], normalize, depth + 1);
    if (found) return found;
  }

  for (const [key, child] of Object.entries(record)) {
    if (visited.has(key)) continue;
    if (child == null || ['string', 'number', 'boolean'].includes(typeof child)) continue;
    const found = extractNestedArtifact(child, normalize, depth + 1);
    if (found) return found;
  }

  return null;
}

export function extractPlanArtifact(value: unknown): PlanArtifact | null {
  return extractNestedArtifact(value, normalizePlanArtifact);
}

export function extractVerificationArtifact(value: unknown): VerificationArtifact | null {
  return extractNestedArtifact(value, normalizeVerificationArtifact);
}

export function extractSubtaskArtifacts(value: unknown): SubtaskRunArtifact[] {
  return extractNestedArtifact(value, normalizeSubtaskArtifacts) ?? [];
}

export function findLatestPlanArtifact(
  messages: ConversationMessage[],
  toolCalls: ToolCallEvent[],
  taskPlan?: unknown,
): PlanArtifact | null {
  const taskRunPlan = extractPlanArtifact(taskPlan);
  if (taskRunPlan) return taskRunPlan;
  for (let i = toolCalls.length - 1; i >= 0; i -= 1) {
    const artifact = extractPlanArtifact(toolCalls[i].artifacts);
    if (artifact) return artifact;
  }
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const artifact = extractPlanArtifact(messages[i].artifacts);
    if (artifact) return artifact;
  }
  return null;
}

export function findLatestVerificationArtifact(
  messages: ConversationMessage[],
  toolCalls: ToolCallEvent[],
  taskArtifacts?: unknown,
): VerificationArtifact | null {
  const taskRunVerification = extractVerificationArtifact(taskArtifacts);
  if (taskRunVerification) return taskRunVerification;
  for (let i = toolCalls.length - 1; i >= 0; i -= 1) {
    const artifact = extractVerificationArtifact(toolCalls[i].artifacts);
    if (artifact) return artifact;
  }
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const artifact = extractVerificationArtifact(messages[i].artifacts);
    if (artifact) return artifact;
  }
  return null;
}

export function findLatestSubtaskArtifacts(
  messages: ConversationMessage[],
  toolCalls: ToolCallEvent[],
  taskArtifacts?: unknown,
  taskEvents: AgentTaskRunEvent[] = [],
): SubtaskRunArtifact[] {
  const merged = new Map<string, SubtaskRunArtifact>();
  const isSubagentTool = (name: string | null | undefined) => matchesSubagentToolName(name);
  const persistedSubagentCallIds = new Set(
    messages.flatMap(message => message.toolCalls)
      .filter(call => isSubagentTool(call.name))
      .map(call => call.id),
  );

  mergeSubtaskArtifacts(merged, extractSubtaskArtifacts(taskArtifacts));
  for (const message of messages) {
    if (message.toolCallId && persistedSubagentCallIds.has(message.toolCallId)) {
      mergeSubtaskArtifacts(merged, extractSubtaskArtifacts(message.artifacts));
    }
  }
  for (const call of toolCalls) {
    if (isSubagentTool(call.toolName)) {
      mergeSubtaskArtifacts(merged, extractSubtaskArtifacts(call.artifacts));
    }
  }
  for (const event of taskEvents) {
    const subtask = subtaskArtifactFromTimelineEvent(event);
    if (subtask) mergeSubtaskArtifacts(merged, [subtask]);
  }

  // Lifecycle activity keeps arriving after the spawn command itself is done.
  // It is the current worker state and must outrank its historical snapshot.
  for (const call of toolCalls) {
    if (isSubagentTool(call.toolName)) mergeSubtaskArtifacts(merged, lifecycleSubtasks(call.activityEvents));
  }

  return [...merged.values()];
}

function lifecycleSubtasks(events: ActivityEvent[] | undefined): SubtaskRunArtifact[] {
  const workers = new Map<string, SubtaskRunArtifact>();
  for (const event of events ?? []) {
    const payload = asRecord(event.payload);
    const envelope = typeof payload?.subagentEvent === 'string' ? payload : asRecord(payload?.detail);
    const agentId = asText(envelope?.agentId);
    const kind = asText(envelope?.subagentEvent);
    if (!agentId || !kind) continue;
    const detail = asRecord(envelope?.detail);
    const result = asRecord(detail?.result);
    const status = kind === 'completed' ? 'completed' : kind === 'failed' ? 'failed' : kind === 'cancelled' ? 'cancelled'
      : ['spawned', 'queued', 'connected'].includes(kind) ? 'running' : null;
    if (!status) continue;
    const worker = normalizeSubtaskRun({
      ...result, id: agentId, agentId, workerId: result?.id, status,
      task: detail?.task ?? result?.task ?? workers.get(agentId)?.task,
      label: detail?.task ?? result?.task ?? workers.get(agentId)?.label ?? agentId,
      role: detail?.role ?? result?.roleName ?? result?.role,
      errorMessage: detail?.errorMessage,
    }, true);
    if (worker) mergeSubtaskArtifacts(workers, [worker]);
  }
  return [...workers.values()];
}

function matchesSubagentToolName(name: string | null | undefined): boolean {
  return matchesToolName(name, [
    'spawn_subagent',
    'spawn_subagent_batch',
    'judge_subagent_results',
    'observe_subagent',
    'wait_subagent',
    'send_subagent_input',
    'cancel_subagent',
    'close_subagent',
  ]);
}

function matchesToolName(
  name: string | null | undefined,
  candidates: string[],
): boolean {
  const normalized = name?.trim().toLowerCase();
  return Boolean(normalized && candidates.includes(normalized));
}
