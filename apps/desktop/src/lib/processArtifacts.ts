import type { ActivityEvent, ArtifactPayload } from '../types/conversation';

type RecordValue = Record<string, unknown>;
const record = (value: unknown): RecordValue | null =>
  value && typeof value === 'object' && !Array.isArray(value) ? value as RecordValue : null;
const text = (value: unknown): string => typeof value === 'string' ? value : '';
const MAX_EVENTS = 128;
const MAX_OUTPUT = 8_192;

export function mergeToolActivityEvents(previous: ActivityEvent[] | undefined, artifacts: ArtifactPayload | undefined): ActivityEvent[] | undefined {
  const root = record(artifacts);
  const activity = record(root?.activity);
  const candidates = root?.kind === 'activityObservation' && Array.isArray(activity?.events)
    ? activity.events : activity ? [activity] : [];
  if (!candidates.length) return previous;
  const events = new Map((previous ?? []).map(event => [`${event.activityId}:${event.seq}`, event]));
  for (const candidate of candidates) {
    const event = record(candidate);
    if (!event || typeof event.activityId !== 'string' || typeof event.seq !== 'number'
      || !Number.isSafeInteger(event.seq) || event.seq < 0 || typeof event.kind !== 'string'
      || typeof event.timestamp !== 'string' || !record(event.payload)) continue;
    const payload = { ...record(event.payload) };
    // Output is a bounded visual tail; the runtime remains the full log owner.
    if (typeof payload.data === 'string') payload.data = payload.data.slice(-MAX_OUTPUT);
    events.set(`${event.activityId}:${event.seq}`, { ...event, payload } as unknown as ActivityEvent);
  }
  return [...events.values()].sort((a, b) => a.seq - b.seq).slice(-MAX_EVENTS);
}

export type ProcessPreviewState = 'running' | 'ready' | 'unhealthy' | 'completed' | 'failed' | 'stopped' | 'unavailable';
export interface ManagedProcessPreview {
  id: string;
  program: string;
  state: ProcessPreviewState;
  readyUrl: string | null;
  output: string;
}

function stateOf(value: unknown): ProcessPreviewState {
  if (value === 'ready' || value === 'unhealthy' || value === 'completed' || value === 'failed') return value;
  if (value === 'exited') return 'completed';
  if (value === 'stopped' || value === 'cancelled' || value === 'superseded') return 'stopped';
  if (value === 'timed_out' || value === 'timedOut') return 'failed';
  if (value === 'orphaned') return 'unavailable';
  return 'running';
}

function loopbackUrl(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  try {
    const url = new URL(value);
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) return null;
    if (url.hostname !== 'localhost' && url.hostname !== '[::1]' && !/^127(?:\.\d{1,3}){3}$/u.test(url.hostname)) return null;
    return url.href;
  } catch { return null; }
}

export function extractManagedProcess(toolName: string, artifacts: ArtifactPayload | undefined, activityEvents?: ActivityEvent[]): ManagedProcessPreview | null {
  if (toolName !== 'run_shell' && toolName !== 'activity_observe') return null;
  const root = record(artifacts);
  const nestedService = root?.kind === 'activityObservation' ? record(root.service) : null;
  const service = root?.kind === 'managedService' ? root
    : nestedService?.kind === 'managedService' ? nestedService : null;
  const observation = root?.kind === 'activityObservation' ? record(root.activity) : null;
  const activityRecord = record(observation?.record) ?? record(root?.activityRecord);
  const events = mergeToolActivityEvents(activityEvents, artifacts) ?? [];
  if (toolName === 'activity_observe' && !service
    && (activityRecord?.ownerTool !== 'run_shell' || activityRecord?.surface !== 'process')) return null;
  const id = text(service?.activityId) || text(activityRecord?.activityId) || events[0]?.activityId;
  if (!id || (!service && !events.length && activityRecord?.ownerTool !== 'run_shell')) return null;
  let state = stateOf(service?.status ?? activityRecord?.state);
  const execution = record(service?.execution);
  if (service?.status === 'exited' && execution && (execution.exitCode !== 0 || execution.timedOut === true)) state = 'failed';
  let readyUrl = loopbackUrl(service?.readyUrl);
  const cursor = typeof service?.cursor === 'number' ? service.cursor
    : typeof observation?.cursor === 'number' ? observation.cursor : -1;
  let output = [text(service?.stdoutTail), text(service?.stderrTail)].filter(Boolean).join('\n');
  for (const event of events) {
    if (event.activityId !== id || event.seq <= cursor) continue;
    if (event.kind === 'stdout_chunk' || event.kind === 'stderr_chunk') output += text(event.payload.data);
    if (event.kind === 'ready_url') {
      readyUrl = loopbackUrl(event.payload.url);
      if (readyUrl) state = 'ready';
    }
    if (['started', 'state_changed', 'completed', 'failed', 'cancelled', 'timed_out', 'superseded'].includes(event.kind)) {
      state = stateOf(event.payload.state ?? event.kind);
    }
  }
  if (!output) {
    output = events.filter(event => event.activityId === id && ['stdout_chunk', 'stderr_chunk'].includes(event.kind))
      .map(event => text(event.payload.data)).join('');
  }
  if (state !== 'ready') readyUrl = null;
  const started = events.find(event => event.activityId === id && event.kind === 'command_started');
  return { id, program: text(service?.program) || text(started?.payload.program) || text(activityRecord?.ownerTool) || 'run_shell', state, readyUrl, output: output.slice(-MAX_OUTPUT) };
}
