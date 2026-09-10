export type LiveProtocol = 'openAiRealtime' | 'geminiLive' | 'qwenRealtime';
export type LivePhase = 'connecting' | 'listening' | 'analyzing' | 'stopping' | 'stopped' | 'error';
export interface LiveEntry { id: string; role: string; text: string; atMs: number; complete: boolean }
export interface LiveMetrics { framesReceived: number; framesSubmitted: number; framesReplaced: number; audioMs: number; lastResponseMs: number | null; omittedEntries: number }
export interface LiveSnapshot {
  id: string; mode: LiveProtocol | 'incremental'; model: string; phase: LivePhase;
  sampleRate: number; startedAt: string; sequence: number; entries: LiveEntry[];
  metrics: LiveMetrics; error: string | null;
}
export interface LiveRecord { snapshot: LiveSnapshot; summary: string | null }
export interface LiveConnection { id: string; name: string; model: string; isDefault: boolean; nativeProtocols: LiveProtocol[]; vision: 'supported' | 'unsupported' | 'unknown' }
export interface StartLiveRequest {
  connectionId: string; protocol: LiveProtocol | null; nativeModel: string | null; nativeEndpoint: string | null;
  microphone: boolean; images: boolean; purpose: string; intervalSeconds: number;
}
export interface LiveRecordRef { id: string; startedAt: string; model: string }
export type LiveEvent = { sessionId: string; sequence: number } & (
  { type: 'state'; phase: LivePhase; error: string | null } |
  { type: 'entry'; entry: LiveEntry } | { type: 'metrics'; metrics: LiveMetrics }
);
export interface LiveTransport {
  connection?(listener: (state: 'connected' | 'reconnecting' | 'closed') => void): () => void;
  isTransientError?(error: unknown): boolean;
  connections(): Promise<LiveConnection[]>;
  start(request: StartLiveRequest): Promise<LiveSnapshot>;
  snapshot(sessionId: string): Promise<LiveSnapshot>;
  frame(sessionId: string, mimeType: string, data: string): Promise<void>;
  audio(sessionId: string, data: Uint8Array): Promise<void>;
  stop(sessionId: string): Promise<LiveSnapshot>;
  summarize(sessionId: string, connectionId: string): Promise<LiveRecord>;
  list(): Promise<LiveRecordRef[]>;
  load(sessionId: string): Promise<LiveRecord>;
  subscribe(listener: (event: LiveEvent) => void): Promise<() => void>;
}
export const liveEnded = (phase: LivePhase) => phase === 'stopped' || phase === 'error';
export function encodeLiveAudio(data: Uint8Array): string {
  let binary = '';
  for (const byte of data) binary += String.fromCharCode(byte);
  return btoa(binary);
}
export function applyLiveEvent(snapshot: LiveSnapshot, event: LiveEvent): LiveSnapshot {
  if (snapshot.id !== event.sessionId || event.sequence <= snapshot.sequence) return snapshot;
  const next = { ...snapshot, sequence: event.sequence };
  if (event.type === 'state') return { ...next, phase: event.phase, error: event.error };
  if (event.type === 'metrics') return { ...next, metrics: event.metrics };
  const entries = [...snapshot.entries];
  const index = entries.findIndex(entry => entry.id === event.entry.id);
  if (index < 0) entries.push(event.entry); else entries[index] = event.entry;
  return { ...next, entries: entries.slice(-160) };
}
