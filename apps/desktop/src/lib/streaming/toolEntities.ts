import type { StreamRoundEvent, ToolCallEvent, TraceEvent } from './protocol';

type RoundRef = Omit<StreamRoundEvent,'toolCalls'> & { toolIds: string[] };
type TraceRef = Exclude<TraceEvent,{ kind: 'tool' }> | { id: string; kind: 'tool'; toolId: string };

/** One owner for tool payloads. Ordered views retain IDs and materialize a
 * structurally shared compatibility projection only when a reader asks for it.
 * Reducers update by ID; they never patch three parallel payload collections. */
export class StreamToolEntities {
  readonly byId = new Map<string,ToolCallEvent>();
  private readonly currentIds = new Map<string,string>();
  private readonly entityKeys = new WeakMap<ToolCallEvent,string>();
  private sequence = 0;
  private activeIds: string[] = [];
  private activeSet = new Set<string>();
  private rounds: RoundRef[] = [];
  private trace: TraceRef[] = [];
  private revision = 0;
  private activeRevision = -1;
  private roundsRevision = -1;
  private traceRevision = -1;
  private activeView: ToolCallEvent[] = [];
  private roundsView: StreamRoundEvent[] = [];
  private traceView: TraceEvent[] = [];
  private readonly roundViews = new Map<string,StreamRoundEvent>();
  private readonly traceViews = new Map<string,TraceEvent>();

  get(callId: string): ToolCallEvent | undefined { const id = this.currentIds.get(callId); return id ? this.byId.get(id) : undefined; }
  key(tool: ToolCallEvent): string | undefined { return this.entityKeys.get(tool); }
  hasActive(callId: string): boolean { return this.activeSet.has(callId); }
  get activeCount(): number { return this.activeIds.length; }
  get roundCount(): number { return this.rounds.length; }
  get traceCount(): number { return this.trace.length; }
  private reference(tool: ToolCallEvent): string {
    const known = this.entityKeys.get(tool);
    if (known) return known;
    const id = `${++this.sequence}:${tool.callId}`;
    this.entityKeys.set(tool,id);
    this.byId.set(id,tool);
    if (!this.activeSet.has(tool.callId)) this.currentIds.set(tool.callId,id);
    this.revision++;
    return id;
  }
  set(tool: ToolCallEvent, entityId?: string): string {
    let id = entityId ?? this.entityKeys.get(tool);
    if (!id) {
      id = this.activeSet.has(tool.callId) ? this.currentIds.get(tool.callId) : undefined;
      if (!id) return this.reference(tool);
    }
    this.entityKeys.set(tool,id);
    if (this.byId.get(id) === tool) return id;
    this.byId.set(id,tool);
    this.revision++;
    return id;
  }

  get toolCalls(): ToolCallEvent[] {
    if (this.activeRevision !== this.revision) {
      this.activeView = this.activeIds.map(id => this.byId.get(id)!);
      this.activeRevision = this.revision;
    }
    return this.activeView;
  }
  set toolCalls(tools: ToolCallEvent[]) {
    this.activeIds = tools.map(tool => this.reference(tool));
    tools.forEach((tool,index) => this.currentIds.set(tool.callId,this.activeIds[index]));
    this.activeSet = new Set(tools.map(tool => tool.callId));
    this.activeView = tools;
    this.activeRevision = this.revision;
  }

  get streamRounds(): StreamRoundEvent[] {
    if (this.roundsRevision !== this.revision) {
      this.roundsView = this.rounds.map(round => {
        const previous = this.roundViews.get(round.id);
        const tools = round.toolIds.map(id => this.byId.get(id)!);
        if (previous && previous.reply === round.reply && previous.thinking === round.thinking && previous.toolCalls.length === tools.length && tools.every((tool,index) => previous.toolCalls[index] === tool)) return previous;
        const next = { id: round.id, thinking: round.thinking, reply: round.reply, toolCalls: tools };
        this.roundViews.set(round.id,next);
        return next;
      });
      this.roundsRevision = this.revision;
    }
    return this.roundsView;
  }
  set streamRounds(rounds: StreamRoundEvent[]) {
    this.rounds = rounds.map(round => ({ id: round.id, thinking: round.thinking, reply: round.reply, toolIds: round.toolCalls.map(tool => this.reference(tool)) }));
    for (const round of rounds) this.roundViews.set(round.id,round);
    const ids = new Set(rounds.map(round => round.id));
    for (const id of this.roundViews.keys()) if (!ids.has(id)) this.roundViews.delete(id);
    this.roundsView = rounds;
    this.roundsRevision = this.revision;
  }

  get traceEvents(): TraceEvent[] {
    if (this.traceRevision !== this.revision) {
      this.traceView = this.trace.map(event => {
        if (event.kind !== 'tool') return event;
        const toolCall = this.byId.get(event.toolId)!;
        const previous = this.traceViews.get(event.id);
        if (previous?.kind === 'tool' && previous.toolCall === toolCall) return previous;
        const next: TraceEvent = { id: event.id, kind: 'tool', toolCall };
        this.traceViews.set(event.id,next);
        return next;
      });
      this.traceRevision = this.revision;
    }
    return this.traceView;
  }
  set traceEvents(events: TraceEvent[]) {
    this.trace = events.map(event => { if (event.kind !== 'tool') return event; return { id: event.id, kind: 'tool', toolId: this.reference(event.toolCall) }; });
    for (const event of events) this.traceViews.set(event.id,event);
    const ids = new Set(events.map(event => event.id));
    for (const id of this.traceViews.keys()) if (!ids.has(id)) this.traceViews.delete(id);
    this.traceView = events;
    this.traceRevision = this.revision;
  }

  /** Reclaim payloads only after all ordered views have released their IDs. */
  prune(): void {
    const retained = new Set(this.activeIds);
    for (const round of this.rounds) for (const id of round.toolIds) retained.add(id);
    for (const event of this.trace) if (event.kind === 'tool') retained.add(event.toolId);
    for (const id of this.byId.keys()) if (!retained.has(id)) this.byId.delete(id);
    for (const [callId,id] of this.currentIds) if (!this.byId.has(id)) this.currentIds.delete(callId);
  }
}
