import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { Archive, ArchiveRestore, Trash2, ArrowDown, ArrowUp, ChevronDown, CircleStop, FolderOpen, Loader2, Plus, Send, Shield, ShieldCheck, SquarePen, Volume2 } from "lucide-react";
import { MobileConversationRow } from './MobileConversationRow';
import { MobileChatHeader, MobileDialog, type MobileChatPresentation } from './MobileChatShell';
import { useTranslation } from "../../i18n";
import type { RemoteChatClient } from "./remoteClient";
import { remoteButton, remoteField } from "./remoteUi";
import { ConnectionModelPicker } from '../models/ConnectionModelPicker';
import type { TurnModelSelection } from '../models/modelChoices';
import { RemoteVoiceInput } from './RemoteVoiceInput';
import { NativeDictationInput } from './NativeDictationInput';
import { nativeSpeechErrorKey } from './nativeSpeechErrors';
import { NexaCombobox } from '../../components/ui/overlay/Combobox';
import { finalAnswerToSpeechText } from '../../lib/autoSpeech';
import { defaultFollowupSuggestions, latestFollowupSuggestions, stripFollowupBlocks } from '../../lib/followupSuggestions';
import { applyVoiceDictationEvent, type VoiceDraftSession } from '../voice/voiceDraftProjection';
import { StreamingMarkdown } from '../../components/chat/StreamingMarkdown';
import { RemoteComposerOptions, defaultComposerSettings, type RemoteComposerSettings } from './RemoteComposerOptions';
import type { AgentRunEvent, ImageAttachment } from '../../types/conversation';
import { enqueueStreamRunEvent, takeNextStreamRunEvent, takeAuthoritativeRunEventSuffix, type StreamEventOrderingState } from '../../lib/streaming/ordering';
import { RemoteRunTimeline, type RemoteTimelineItem } from './RemoteRunTimeline';
import { mergeRemoteHistory } from './remoteHistory';
import { recoveredDeltaSuffix, utf8ByteLength } from '../../lib/streaming/blockTrace';

// Durable fragments are at most 64 KiB. Retain enough UTF-16 units to validate
// that entire overlap even after the independently bounded visible prefix.
function replayTail(text: string): string {
  const tail = text.slice(-65_538);
  const first = tail.charCodeAt(0);
  return first >= 0xdc00 && first <= 0xdfff ? tail.slice(1) : tail;
}

interface Connection {
  id: string;
  name: string;
  model: string;
  isDefault: boolean;
}
interface Conversation {
  id: string;
  title: string;
  model: string;
}
interface ChatMessage {
  id: string;
  role: string;
  content: string;
  totalChars: number;
  sortOrder: number;
  intermediate?: boolean;
  thinking?: string;
  thinkingTotalChars?: number;
}
interface MessagePage {
  messages: ChatMessage[];
  beforeOrder: number | null;
}
type RunEvent = AgentRunEvent;
interface RunPage {
  run: { id: string; status: string; phase?: string; finalMessageId?: string | null; errorMessage?: string | null } | null;
  events: RunEvent[];
  hasMore: boolean;
  nextSequence: number | null;
  durableHighWater?: number;
}
interface Approval {
  runId: string;
  request: {
    id: string;
    toolName: string;
    reason: string;
    targetValue: string;
    argumentsPreview: string;
    choices?: string[];
  };
}
interface Question {
  id: string;
  question: string;
  type: string;
  options?: { label: string; description?: string }[];
}
interface Interaction {
  interactionId: string;
  title: string;
  description?: string;
  resumeToken: string;
  questions: Question[];
}

export function RemoteChat({
  client,
  initialDraft,
  onDraftConsumed,
  emptyState,
  mobile,
}: {
  client: RemoteChatClient;
  initialDraft: string;
  onDraftConsumed: () => void;
  emptyState?: ReactNode;
  mobile?: MobileChatPresentation;
}) {
  const { t, locale } = useTranslation();
  const [speakingId, setSpeakingId] = useState<string | null>(null);
  const speechGeneration = useRef(0);
  useEffect(() => () => { speechGeneration.current++; void client.nativeSpeech?.stop().catch(() => {}); }, [client]);
  const speak = async (message: ChatMessage) => {
    const speech = client.nativeSpeech; if (!speech) return;
    const generation = ++speechGeneration.current;
    const stopping = speakingId === message.id;
    setSpeakingId(null); await speech.stop().catch(() => {});
    if (stopping || generation !== speechGeneration.current) return;
    setSpeakingId(message.id);
    try {
      const parts = [message.content];
      let offset = [...message.content].length;
      while (offset < message.totalChars) {
        const page = await client.rpc<{ content: string; totalChars: number }>('chat.message', {
          conversationId, messageId: message.id, offset,
        });
        if (generation !== speechGeneration.current) return;
        const count = [...page.content].length;
        if (!count) throw new Error('speech_failed');
        parts.push(page.content); offset += count;
      }
      if (generation === speechGeneration.current) await speech.speak(finalAnswerToSpeechText(parts.join('')), locale);
    }
    catch (error) { if (generation === speechGeneration.current) setError(t(nativeSpeechErrorKey(error))); }
    finally { if (generation === speechGeneration.current) setSpeakingId(null); }
  };
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);
  const [optionsOpen, setOptionsOpen] = useState(false);
  const contentScroll = useRef<HTMLDivElement>(null);
  const composerInput = useRef<HTMLTextAreaElement>(null);
  const followOutput = useRef(true);
  const [connections, setConnections] = useState<Connection[]>([]);
  const [connectionId, setConnectionId] = useState("");
  const [modelSelection, setModelSelection] = useState<TurnModelSelection | null>(null);
  const [catalogRevision, setCatalogRevision] = useState(0);
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const listGeneration = useRef(0);
  const archiveGeneration = useRef(0);
  const [cursor, setCursor] = useState<string | null>(null);
  const [conversationId, setConversationId] = useState(
    () =>
      localStorage.getItem(
        `nexa.remote.chat.${client.paired.manifest.serverId}`,
      ) || "",
  );
  const selectedConversation = useRef(conversationId);
  useEffect(() => {
    speechGeneration.current++; setSpeakingId(null);
    void client.nativeSpeech?.stop().catch(() => {});
  }, [client, conversationId]);
  const selectConversation = (id: string) => {
    selectedConversation.current = id;
    setConversationId(id);
  };
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [beforeOrder, setBeforeOrder] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const draftRef = useRef(draft); draftRef.current = draft;
  const voiceDraft = useRef<VoiceDraftSession | null>(null);
  const [dictating, setDictating] = useState(false);
  const [attachments, setAttachments] = useState<ImageAttachment[]>([]);
  const [composerSettings, setComposerSettings] = useState<RemoteComposerSettings>(() => {
    const saved = client.approvalModeAvailable ? localStorage.getItem(`nexa.approval.${client.paired.manifest.serverId}`) : null;
    return saved === 'ask' || saved === 'allow_all' || saved === 'deny_all' ? { ...defaultComposerSettings, toolApprovalMode: saved } : defaultComposerSettings;
  });
  useEffect(() => { if (client.approvalModeAvailable) localStorage.setItem(`nexa.approval.${client.paired.manifest.serverId}`, composerSettings.toolApprovalMode ?? 'ask'); }, [client, composerSettings.toolApprovalMode]);
  const [loadingAttachments, setLoadingAttachments] = useState(false);
  const [sending, setSending] = useState(false);
  const sendLock = useRef(false);
  const [archived, setArchived] = useState<Conversation[]>([]);
  const [archivedCursor, setArchivedCursor] = useState<string | null>(null);
  const [archivesOpen, setArchivesOpen] = useState(false);
  const [deleteArchivedIds, setDeleteArchivedIds] = useState<string[] | null>(null);
  const [manageTarget, setManageTarget] = useState<{ item: Conversation; action: "archive" | "delete" } | null>(null);
  const [managing, setManaging] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [timeline, setTimeline] = useState<RemoteTimelineItem[]>([]);
  const [finalMessageId, setFinalMessageId] = useState<string | null>(null);
  const [runStatus, setRunStatus] = useState('');
  const [syncingHistory, setSyncingHistory] = useState(false);
  const [progress, setProgress] = useState("");
  const [running, setRunning] = useState(false);
  useLayoutEffect(() => {
    if (mobile && contentScroll.current && followOutput.current) contentScroll.current.scrollTop = contentScroll.current.scrollHeight;
  }, [mobile, messages, timeline]);
  useLayoutEffect(() => {
    if (!mobile || !composerInput.current) return;
    composerInput.current.style.height = 'auto';
    composerInput.current.style.height = `${Math.min(160, composerInput.current.scrollHeight)}px`;
  }, [mobile, draft]);
  const [approvals, setApprovals] = useState<Approval[]>([]);
  const [interactions, setInteractions] = useState<Interaction[]>([]);
  const [answers, setAnswers] = useState<
    Record<string, Record<string, string[]>>
  >({});
  const scope = useRef(0);
  const refreshRef = useRef<() => void>(() => {});
  const pendingSend = useRef<{
    message: string;
    id: string;
    conversationId: string;
    connectionId: string;
    modelSelection: TurnModelSelection | null;
    attachments: ImageAttachment[];
    settings: RemoteComposerSettings;
  } | null>(null);
  const errorText = (error: unknown) =>
    setError(error instanceof Error ? error.message : String(error));
  const list = useCallback(
    async (before: string | null = null) => {
      const generation = ++listGeneration.current;
      const result = await client.rpc<{
        items: Conversation[];
        nextCursor: string | null;
      }>("chat.list", { before });
      if (generation !== listGeneration.current) return;
      setConversations((current) =>
        before
          ? [
              ...current,
              ...result.items.filter(
                (item) => !current.some((old) => old.id === item.id),
              ),
            ]
          : result.items,
      );
      setCursor(result.nextCursor);
    },
    [client],
  );
  useEffect(() => {
    let active = true;
    const refresh = () => Promise.all([client.rpc<Connection[]>("connections.list"), list()])
      .then(([items]) => {
        if (active) {
          setConnections(items);
          setConnectionId(
            (current) =>
              (items.some(item => item.id === current) ? current : '') ||
              items.find((item) => item.isDefault)?.id ||
              items[0]?.id ||
              "",
          );
        }
      })
      .catch(errorText)
      .finally(() => {
        if (active) setLoading(false);
      });
    void refresh();
    const unsubscribe = client.subscribe(event => {
      if (event.event === 'connections:changed') { setCatalogRevision(value=>value+1); void refresh(); }
      if (event.event === 'conversations:changed') {
        listGeneration.current++; archiveGeneration.current++; setArchived([]); setArchivedCursor(null);
        if (event.payload?.ids?.includes(selectedConversation.current) && event.payload?.action !== 'restore') {
          selectConversation(''); pendingSend.current = null;
          localStorage.removeItem(`nexa.remote.chat.${client.paired.manifest.serverId}`);
        }
        void list().catch(errorText);
      }
    });
    return () => {
      active = false;
      unsubscribe();
    };
  }, [client, list]);
  useEffect(() => {
    if (initialDraft) {
      setDraft(initialDraft);
      onDraftConsumed();
    }
  }, [initialDraft, onDraftConsumed]);
  useEffect(() => {
    const generation = ++scope.current;
    let stopped = false;
    let syncing = false;
    let syncAgain = false;
    let readNeeded = true;
    let historyLoaded = false;
    let runId: string | null = null;
    let finalId: string | null = null;
    let status = '';
    let textReplayAttempts = 0;
    const ordering: StreamEventOrderingState = {
      _orderedRunId: null, _lastEventSeq: 0, _pendingRunEvents: new Map(),
    };
    const blocks = new Map<string, { text: string; bytes: number; tail: string }>();
    const items = new Map<string, RemoteTimelineItem>();
    const encoder = new TextEncoder();
    let paint: ReturnType<typeof setTimeout> | null = null;
    const retired = new Set<string>();
    const terminal = new Set<string>();
    setTimeline([]); setFinalMessageId(null); setRunStatus('');
    setProgress(''); setRunning(false); setMessages([]);
    setApprovals([]); setInteractions([]); setBeforeOrder(null);
    if (!conversationId) return;
    localStorage.setItem(`nexa.remote.chat.${client.paired.manifest.serverId}`, conversationId);
    const valid = () => !stopped && scope.current === generation && selectedConversation.current === conversationId;
    const publish = () => {
      if (paint) return;
      paint = setTimeout(() => {
        paint = null;
        if (valid()) setTimeline([...items.values()].sort((a, b) => a.sequence - b.sequence));
      }, 60);
    };
    const changeRun = (id: string) => {
      if (runId === id) return;
      if (runId) retired.add(runId);
      runId = id; finalId = null; status = ''; textReplayAttempts = 0;
      ordering._orderedRunId = id; ordering._lastEventSeq = 0; ordering._pendingRunEvents.clear();
      blocks.clear(); items.clear(); readNeeded = true;
      setTimeline([]); setFinalMessageId(null); setProgress('');
    };
    const updateStatus = (next: string, phase?: string) => {
      if (runId && terminal.has(runId) && !['completed', 'cancelled', 'failed', 'timed_out'].includes(next)) return;
      const effective = phase === 'paused' || phase === 'awaiting_user_input' ? phase : next;
      if (status !== effective) readNeeded = true;
      status = effective; setRunStatus(effective);
      setRunning(['queued', 'running', 'waiting_approval', 'cancelling'].includes(effective));
      if (runId && ['completed', 'cancelled', 'failed', 'timed_out'].includes(effective)) terminal.add(runId);
    };
    const read = async () => {
      const expectedRun = runId;
      const result = await client.rpc<MessagePage>('chat.read', { conversationId });
      if (!valid() || runId !== expectedRun) return;
      const firstLoad = !historyLoaded;
      historyLoaded = true;
      setMessages(current => mergeRemoteHistory(current, result.messages));
      if (firstLoad) setBeforeOrder(result.beforeOrder);
      // Only the canonical message for THIS run retires its live answer. A
      // previous assistant message or a temporarily missing final cannot do so.
      if (finalId && result.messages.some(message => message.id === finalId)) setFinalMessageId(finalId);
      else if (status === 'completed') readNeeded = true;
    };
    const questions = async () => {
      const expectedRun = runId;
      const [nextApprovals, nextInteractions] = await Promise.all([
        client.rpc<Approval[]>('approvals.list'),
        client.rpc<Interaction[]>('interactions.list', { conversationId }),
      ]);
      if (valid() && runId === expectedRun) {
        setApprovals(nextApprovals.filter(item => item.runId === runId));
        setInteractions(nextInteractions);
      }
    };
    const apply = (event: RunEvent) => {
      if (event.visibility === 'user' && event.label) setProgress(event.label);
      const payload = (event.payload ?? {}) as Record<string, any>;
      if (event.visibility === 'user' && event.kind.startsWith('tool') && payload.run?.callId) {
        const id = `tool:${payload.run.callId}`;
        const previous = items.get(id);
        items.set(id, { id, kind: 'tool', sequence: previous?.sequence ?? event.eventSeq,
          tool: { ...(previous?.kind === 'tool' ? previous.tool : {}), ...payload.run } });
        publish();
      }
      if ((event.kind === 'outputDelta' || event.kind === 'outputSnapshot') &&
          ['answer', 'thinking'].includes(payload.channel) && typeof payload.blockId === 'string') {
        const id = `${payload.channel}:${payload.blockId}`;
        const previous = blocks.get(id) ?? { text: '', bytes: 0, tail: '' };
        const delta = typeof payload.delta === 'string' ? payload.delta : '';
        const suffix = event.kind === 'outputSnapshot' ? String(payload.text ?? payload.content ?? '')
          : previous.bytes === payload.offset ? delta
          : recoveredDeltaSuffix(previous.tail, payload.offset, previous.bytes, delta,
            previous.bytes - utf8ByteLength(previous.tail));
        if (suffix !== null) {
          const text = suffix;
          const next = event.kind === 'outputSnapshot'
            ? { text: text.slice(0, 128_000), bytes: encoder.encode(text).byteLength, tail: replayTail(text) }
            : { text: (previous.text + text).slice(0, 128_000), bytes: previous.bytes + encoder.encode(text).byteLength, tail: replayTail(previous.tail + text) };
          blocks.set(id, next);
          const priorItem = items.get(id);
          items.set(id, { id, kind: payload.channel, sequence: priorItem?.sequence ?? event.eventSeq, text: next.text });
          publish();
        } else if (textReplayAttempts++ === 0) {
          // A byte-offset mismatch needs a full replay; skipping it permanently
          // would make the phone look current while silently dropping text.
          ordering._lastEventSeq = 0;
          ordering._pendingRunEvents.clear();
          blocks.clear(); items.clear();
          syncAgain = true;
        }
      }
      if (event.kind === 'streamReset' && payload.discardSample) {
        blocks.clear();
        for (const [id, item] of items) if (item.kind !== 'tool') items.delete(id);
        publish();
      }
      if (event.kind === 'done' || event.kind === 'error') {
        if (typeof payload.assistantMessageId === 'string') finalId = payload.assistantMessageId;
        const terminalStatus = String(payload.status || event.status || (event.kind === 'done' ? 'completed' : 'failed'));
        updateStatus(terminalStatus);
        readNeeded = true;
        if (terminalStatus === 'cancelled') setError('');
        else if (event.kind === 'error' && typeof payload.message === 'string') setError(payload.message);
        if (!syncing) void sync();
        void list().catch(() => {});
      } else if (event.phase === 'paused' || event.phase === 'awaiting_user_input' || event.phase === 'approval') {
        updateStatus(event.phase === 'approval' ? 'waiting_approval' : event.phase);
      } else if (event.kind === 'status' && ['queued', 'running', 'recovering'].includes(event.status ?? '')) updateStatus('running');
      if (['approvalRequested', 'approvalResolved', 'interactionRequested'].includes(event.kind)) void questions().catch(() => {});
      // Bound presentation independently of the durable history cursor.
      while (items.size > 128) items.delete(items.keys().next().value!);
    };
    const receive = (event: RunEvent) => {
      if (!valid() || retired.has(event.runId)) return;
      // A delayed event can belong to an older run we never observed. Let the
      // desktop select the current run instead of reviving whichever arrives.
      if (runId !== event.runId) { void sync(); return; }
      const admission = enqueueStreamRunEvent(ordering, event);
      if (!admission.accepted) return;
      let next: AgentRunEvent | null;
      while ((next = takeNextStreamRunEvent(ordering)) !== null) apply(next);
      if (ordering._pendingRunEvents.size > 256) ordering._pendingRunEvents.clear();
      if (admission.missingRange || syncAgain) void sync();
    };
    const sync = async () => {
      if (!valid()) return;
      if (syncing) { syncAgain = true; return; }
      syncing = true;
      setSyncingHistory(true);
      try {
        let cursor = ordering._lastEventSeq;
        let highWater: number | undefined;
        for (;;) {
          const requestedRun = runId;
          const page = await client.rpc<RunPage>('chat.resume', {
            conversationId, runId: requestedRun, afterSequence: cursor, durableHighWater: highWater,
          });
          if (!valid()) return;
          if (!page.run) { updateStatus(''); break; }
          if (retired.has(page.run.id)) { syncAgain = false; return; }
          if (runId !== page.run.id) changeRun(page.run.id);
          if (requestedRun !== page.run.id) { cursor = 0; highWater = undefined; }
          highWater ??= page.durableHighWater;
          if (page.run.finalMessageId) finalId = page.run.finalMessageId;
          updateStatus(page.run.status, page.run.phase);
          if (page.run.status === 'cancelled') setError('');
          else if (page.run.errorMessage) setError(page.run.errorMessage);
          // The durable ledger omits ephemeral previews. Only a canonical
          // replay page may advance over those holes; live events stay ordered.
          const through = page.hasMore ? page.nextSequence ?? cursor
            : highWater ?? page.nextSequence ?? Math.max(cursor, ...page.events.map(event => event.eventSeq));
          for (const event of takeAuthoritativeRunEventSuffix(ordering, page.run.id, page.events, {
            includeLivePending: true, authoritativeThroughEventSeq: through,
          })) apply(event);
          if (!page.hasMore) break;
          const nextCursor = page.nextSequence;
          if (nextCursor == null || nextCursor <= cursor) throw new Error('Remote history did not advance its cursor');
          cursor = nextCursor;
        }
        const shouldRead = readNeeded;
        readNeeded = false;
        await Promise.all([shouldRead ? read() : Promise.resolve(), questions()]);
      } catch (error) {
        readNeeded = true;
        if (valid() && client.state.phase === 'connected') errorText(error);
      } finally {
        syncing = false;
        if (valid()) setSyncingHistory(false);
        if (syncAgain && valid()) { syncAgain = false; void sync(); }
      }
    };
    refreshRef.current = () => { readNeeded = true; void sync(); };
    const off = client.subscribe(event => {
      if (event.event === 'agent://run-event' && event.payload.conversationId === conversationId) receive(event.payload.runEvent);
      else if (event.event === 'connection:resync') {
        readNeeded = true;
        void list().catch(() => {}); void sync();
      }
    });
    const foreground = () => {
      if (document.visibilityState !== 'hidden') { readNeeded = true; void sync(); }
    };
    document.addEventListener('visibilitychange', foreground);
    window.addEventListener('pageshow', foreground);
    void sync();
    const timer = setInterval(() => {
      if (client.state.phase === 'connected' && document.visibilityState !== 'hidden') void sync();
    }, 3000);
    return () => {
      stopped = true; off(); clearInterval(timer);
      document.removeEventListener('visibilitychange', foreground);
      window.removeEventListener('pageshow', foreground);
      if (paint) clearTimeout(paint);
    };
  }, [client, conversationId, list]);
  async function create() {
    if (sendLock.current) return;
    // A draft is persisted only on its first send. Repeated New taps reuse it.
    if (!conversationId && !draftRef.current.trim() && !attachments.length) { setDrawerOpen(false); composerInput.current?.focus(); return; }
    selectConversation(""); pendingSend.current = null;
    setDraft(""); draftRef.current = ""; setAttachments([]);
    localStorage.removeItem(`nexa.remote.chat.${client.paired.manifest.serverId}`);
    setDrawerOpen(false); composerInput.current?.focus();
  }
  async function listArchived(before: string | null = null) {
    const generation = ++archiveGeneration.current;
    const page = await client.rpc<{ items: Conversation[]; nextCursor: string | null }>('chat.archived', { before });
    if (generation !== archiveGeneration.current) return;
    setArchived(current => before ? [...current, ...page.items.filter(item => !current.some(old => old.id === item.id))] : page.items);
    setArchivedCursor(page.nextCursor);
  }
  async function restoreConversation(id: string) {
    if (managing) return;
    listGeneration.current++; archiveGeneration.current++;
    setManaging(true); setError('');
    try { await client.rpc('chat.restore', { conversationId: id }); await Promise.all([list(), listArchived()]); }
    catch (error) { errorText(error); }
    finally { setManaging(false); }
  }
  async function prepareDeleteArchived() {
    if (managing) return;
    setManaging(true); setError('');
    try {
      const ids = new Set<string>(), cursors = new Set<string>();
      let before: string | null = null;
      do {
        const page: {items: Conversation[]; nextCursor: string | null} = await client.rpc('chat.archived', { before });
        page.items.forEach(item => ids.add(item.id));
        before = page.nextCursor;
        if (before && cursors.has(before)) throw new Error('Archived history did not advance its cursor');
        if (before) cursors.add(before);
      } while (before);
      setDeleteArchivedIds([...ids]);
    } catch (error) { errorText(error); }
    finally { setManaging(false); }
  }
  async function deleteArchived() {
    if (!deleteArchivedIds || managing) return;
    setManaging(true); setError('');
    listGeneration.current++; archiveGeneration.current++;
    const remaining = [...deleteArchivedIds];
    try {
      while (remaining.length) {
        const id = remaining[0];
        await client.rpc('chat.deleteArchived', { conversationId: id });
        remaining.shift();
        if (selectedConversation.current === id) { selectConversation(''); pendingSend.current = null; }
      }
      setDeleteArchivedIds(null);
    } catch (error) { setDeleteArchivedIds(remaining); errorText(error); }
    finally { await Promise.all([list(), listArchived()]).catch(errorText); setManaging(false); }
  }
  async function manageConversation() {
    if (!manageTarget || managing) return;
    listGeneration.current++; archiveGeneration.current++;
    setManaging(true); setError("");
    try {
      await client.rpc(`chat.${manageTarget.action}`, { conversationId: manageTarget.item.id });
      if (selectedConversation.current === manageTarget.item.id) {
        selectConversation(""); pendingSend.current = null;
        localStorage.removeItem(`nexa.remote.chat.${client.paired.manifest.serverId}`);
      }
      setManageTarget(null); await list(); if (archivesOpen) await listArchived();
    } catch (error) { errorText(error); }
    finally { setManaging(false); }
  }
  async function send(message = draft) {
    if (sendLock.current || running || dictating || loadingAttachments || (!message.trim() && !attachments.length) || !connectionId) return;
    const generation = scope.current;
    sendLock.current = true;
    setSending(true);
    setError("");
    try {
      let id = conversationId;
      if (!id) {
        const conversation = await client.rpc<Conversation>("chat.create", {
          connectionId,
          modelSelection,
        });
        id = conversation.id;
        if (
          scope.current === generation &&
          selectedConversation.current === conversationId
        )
          selectConversation(id);
        setConversations((current) => [
          conversation,
          ...current.filter((item) => item.id !== conversation.id),
        ]);
      }
      const previous = pendingSend.current;
      const request =
        previous &&
        previous.message === message &&
        previous.conversationId === id &&
        previous.connectionId === connectionId &&
        previous.attachments === attachments && previous.settings === composerSettings &&
        JSON.stringify(previous.modelSelection) === JSON.stringify(modelSelection)
          ? previous
          : {
              message,
              id: crypto.randomUUID(),
              conversationId: id,
              connectionId,
              modelSelection,
              attachments,
              settings: composerSettings,
            };
      pendingSend.current = request;
      await client.rpc("chat.start", {
        conversationId: id,
        connectionId,
        message: request.message,
        idempotencyKey: request.id,
        modelSelection: request.modelSelection,
        attachments: request.attachments,
        ...request.settings,
      });
      if (pendingSend.current === request) pendingSend.current = null;
      if (selectedConversation.current === id) {
        setDraft((current) => (current === request.message ? "" : current));
        setAttachments(current => current === request.attachments ? [] : current);
        setRunning(true);
        refreshRef.current();
      }
    } catch (error) {
      errorText(error);
    } finally {
      sendLock.current = false;
      setSending(false);
    }
  }
  async function act(method: string, params: unknown) {
    setError("");
    try {
      await client.rpc(method, params);
      refreshRef.current();
    } catch (error) {
      errorText(error);
    }
  }
  async function older() {
    const generation = scope.current;
    const valid = () =>
      scope.current === generation &&
      selectedConversation.current === conversationId;
    try {
      const page = await client.rpc<MessagePage>("chat.read", {
        conversationId,
        beforeOrder,
      });
      if (!valid()) return;
      setMessages((current) => {
        const ids = new Set(current.map((message) => message.id));
        return [
          ...page.messages.filter((message) => !ids.has(message.id)),
          ...current,
        ];
      });
      setBeforeOrder((current) =>
        current === beforeOrder ? page.beforeOrder : current,
      );
    } catch (error) {
      if (valid()) errorText(error);
    }
  }
  async function more(message: ChatMessage, channel: 'content' | 'thinking' = 'content') {
    const generation = scope.current;
    const valid = () =>
      scope.current === generation &&
      selectedConversation.current === conversationId;
    try {
      const page = await client.rpc<{ content: string; totalChars: number }>(
        "chat.message",
        {
          conversationId,
          messageId: message.id,
          offset: [...(message[channel] ?? '')].length,
          ...(channel === 'thinking' ? { channel } : {}),
        },
      );
      if (!valid()) return;
      setMessages((current) =>
        current.map((item) =>
          item.id === message.id && item[channel] === message[channel]
            ? { ...item, [channel]: (item[channel] ?? '') + page.content }
            : item,
        ),
      );
    } catch (error) {
      if (valid()) errorText(error);
    }
  }
  const lastMessage = messages[messages.length - 1];
  const followups = !running && !sending && (!runStatus || runStatus === 'completed') && !draft && !attachments.length && lastMessage && [...lastMessage.content].length >= lastMessage.totalChars ? latestFollowupSuggestions(messages, defaultFollowupSuggestions(t)) : null;
  const providerControls = <>
      {connections.length > 0 && (mobile ? <NexaCombobox ariaLabel={t('remote.providerConnection')} className={remoteField} value={connectionId} onValueChange={id=>{setConnectionId(id);setModelSelection(null);}} options={connections.map(item=>({value:item.id,label:item.name,description:item.model}))} searchPlaceholder={t('settings.modelSearchPlaceholder')} emptyLabel={t('settings.modelSearchNoResults')}/> : <select
        aria-label={t("remote.providerConnection")}
        className={remoteField}
        value={connectionId}
        onChange={(event) => { setConnectionId(event.target.value); setModelSelection(null); }}
      >
        {connections.map((item) => (
          <option key={item.id} value={item.id}>
            {item.name} 路 {item.model}
          </option>
        ))}
      </select>)}
      {client.modelPickerAvailable !== false && <ConnectionModelPicker key={catalogRevision} connectionId={connectionId} defaultModel={connections.find(item => item.id === connectionId)?.model || ''}
        value={modelSelection} onChange={setModelSelection} load={client.models} label={t('remote.model')} disabled={sending} />}
    </>;
  const conversationControls = <>
      <div className="flex gap-2">
        <select
          aria-label={t("remote.conversation")}
          className={`${remoteField} min-w-0 flex-1`}
          value={conversationId}
          onChange={(event) => {
            selectConversation(event.target.value);
            pendingSend.current = null;
          }}
        >
          <option value="">{t("remote.newChat")}</option>
          {conversations.map((item) => (
            <option key={item.id} value={item.id}>
              {item.title || t('remote.newChat')}
            </option>
          ))}
        </select>
        <button
          className={remoteButton}
          aria-label={t("remote.newChat")}
          onClick={() => void create()}
          disabled={sending || !connectionId}
        >
          <Plus size={18} />
        </button>
      </div>
      {cursor && (
        <button
          className={remoteButton}
          onClick={() => void list(cursor).catch(errorText)}
        >
          {t("remote.moreChats")}
        </button>
      )}
    </>;
  return (
    <main
      className={mobile ? "native-chat" : "mx-auto flex w-full max-w-3xl flex-1 flex-col gap-5 p-4 pb-8"}
      data-testid="remote-chat"
    >
      {mobile ? <><MobileChatHeader center={mobile.heading} onMenu={() => setDrawerOpen(true)} onNew={() => void create()} disabled={sending || !connectionId}/>
        <MobileDialog open={drawerOpen} side title={t('remote.conversation')} onClose={() => setDrawerOpen(false)}><button className="mobile-drawer-new" disabled={sending || !connectionId} onClick={() => { setDrawerOpen(false); void create(); }}><SquarePen size={18}/>{t('remote.newChat')}</button><nav className="mobile-history">{conversations.map(item => <MobileConversationRow key={item.id} onArchive={client.conversationManagementAvailable ? () => setManageTarget({ item, action: 'archive' }) : undefined}><button className={item.id === conversationId ? 'selected' : ''} onClick={() => { selectConversation(item.id); pendingSend.current = null; followOutput.current = true; setDrawerOpen(false); }}>{item.title || t('remote.newChat')}</button>{client.conversationManagementAvailable && <><button className="mobile-icon-button" aria-label={`${t('chat.archive')}: ${item.title || t('remote.newChat')}`} onClick={() => setManageTarget({ item, action: 'archive' })}><Archive size={16}/></button><button className="mobile-icon-button" aria-label={`${t('common.delete')}: ${item.title || t('remote.newChat')}`} onClick={() => setManageTarget({ item, action: 'delete' })}><Trash2 size={16}/></button></>}</MobileConversationRow>)}</nav>{cursor && <button onClick={() => void list(cursor).catch(errorText)}>{t('remote.moreChats')}</button>}{client.conversationManagementAvailable && <details open={archivesOpen} onToggle={event => { const open = event.currentTarget.open; setArchivesOpen(open); if (open) void listArchived().catch(errorText); }}><summary className="mobile-add-connection">{t('chat.archivedConversations')}</summary>{!!archived.length && <button className="mobile-forget" disabled={managing} onClick={() => void prepareDeleteArchived()}><Trash2 size={16}/>{t('chat.deleteAll')}</button>}{error && <p role="alert" className="mobile-error">{error}</p>}<nav className="mobile-history">{archived.map(item => <div key={item.id} className="mobile-conversation-row"><span className="min-w-0 flex-1 truncate">{item.title || t('remote.newChat')}</span><button disabled={managing} className="mobile-icon-button" aria-label={`${t('chat.unarchive')}: ${item.title}`} onClick={() => void restoreConversation(item.id)}><ArchiveRestore size={16}/></button><button disabled={managing} className="mobile-icon-button" aria-label={`${t('common.delete')}: ${item.title}`} onClick={() => setManageTarget({ item, action: 'delete' })}><Trash2 size={16}/></button></div>)}</nav>{archivedCursor && <button onClick={() => void listArchived(archivedCursor).catch(errorText)}>{t('remote.moreChats')}</button>}</details>}<div className="mobile-drawer-footer" onClick={() => setDrawerOpen(false)}>{mobile.drawerFooter}</div></MobileDialog>
        <MobileDialog open={!!manageTarget} title={t(manageTarget?.action === 'archive' ? 'chat.archive' : 'common.delete')} onClose={() => { if (!managing) setManageTarget(null); }}>{error && <p role="alert" className="mobile-error">{error}</p>}<p>{manageTarget?.item.title || t('remote.newChat')}</p><p className="mobile-section-hint">{t(manageTarget?.action === 'archive' ? 'remote.archiveChatHint' : 'remote.deleteChatHint')}</p><div className="mobile-confirm-actions"><button disabled={managing} onClick={() => setManageTarget(null)}>{t('common.cancel')}</button><button className="mobile-primary" disabled={managing} onClick={() => void manageConversation()}>{t(manageTarget?.action === 'archive' ? 'chat.archive' : 'common.delete')}</button></div></MobileDialog>
        <MobileDialog open={deleteArchivedIds !== null} title={t('chat.deleteAll')} onClose={() => { if (!managing) setDeleteArchivedIds(null); }}><p>{t('chat.archivedConversations')}</p><p className="mobile-section-hint">{t('chat.deleteAllConfirmDesc', { count: deleteArchivedIds?.length ?? 0 })}</p>{error && <p role="alert" className="mobile-error">{error}</p>}<div className="mobile-confirm-actions"><button disabled={managing} onClick={() => setDeleteArchivedIds(null)}>{t('common.cancel')}</button><button className="mobile-primary" disabled={managing || !deleteArchivedIds?.length} onClick={() => void deleteArchived()}>{t('chat.deleteAll')}</button></div></MobileDialog>
        <MobileDialog open={modelOpen} title={t('remote.model')} onClose={() => setModelOpen(false)}><div className="mobile-model-controls">{providerControls}</div></MobileDialog>
      </> : <>{conversationControls}{providerControls}</>}
      <div ref={contentScroll} className={mobile ? 'native-chat-content' : 'contents'} onScroll={mobile ? event => { const view = event.currentTarget; followOutput.current = view.scrollHeight - view.scrollTop - view.clientHeight < 80; } : undefined}>
      {error && (
        <p
          role="alert"
          className="rounded-xl border border-danger/30 bg-danger/5 p-3 text-sm text-danger"
        >
          {error}
        </p>
      )}
      {loading && <Loader2 className="mx-auto animate-spin text-accent" />}
      {beforeOrder !== null && (
        <button className={remoteButton} onClick={() => void older()}>
          {t("remote.older")}
        </button>
      )}
      <div className="remote-transcript" aria-live="polite" aria-busy={syncingHistory}>
        {messages.filter(message => message.id !== finalMessageId).map(message => (
          <RemoteChatMessage key={message.id} message={message} onMore={() => void more(message)} onMoreThinking={() => void more(message, 'thinking')} onSpeak={client.nativeSpeech ? () => void speak(message) : undefined} speaking={speakingId === message.id}/>
        ))}
        <RemoteRunTimeline items={timeline} running={running} settled={Boolean(finalMessageId)} />
        {messages.filter(message => message.id === finalMessageId).map(message => (
          <RemoteChatMessage key={message.id} message={message} onMore={() => void more(message)} onMoreThinking={() => void more(message, 'thinking')} onSpeak={client.nativeSpeech ? () => void speak(message) : undefined} speaking={speakingId === message.id}/>
        ))}
      </div>
      {runStatus && (
        <div role="status" data-testid="remote-run-status" className="remote-run-status">
          {running && <Loader2 size={14} className="shrink-0 animate-spin text-accent" />}
          <span className="font-medium">{
            runStatus === 'completed' ? t('remote.runCompleted')
              : runStatus === 'cancelled' ? t('remote.runCancelled')
                : ['failed', 'timed_out'].includes(runStatus) ? t('remote.runFailed')
                  : runStatus === 'paused' ? t('remote.runPaused')
                    : ['awaiting_user_input', 'waiting_input'].includes(runStatus) ? t('remote.runWaitingInput')
                      : runStatus === 'waiting_approval' ? t('remote.runWaitingApproval')
                        : t('remote.runRunning')
          }</span>
          {running && progress && progress !== t('remote.runRunning') && <span className="min-w-0 flex-1 truncate text-text-tertiary">{progress}</span>}
          {runStatus === 'paused' && client.resumeAvailable && <button className={remoteButton} onClick={() => void act('chat.continue', { conversationId, connectionId, modelSelection, ...composerSettings })}>{t('chat.resumeTask')}</button>}
        </div>
      )}
      {approvals.map((item) => (
        <section
          key={item.request.id}
          className="space-y-3 rounded-2xl border border-amber-400/30 bg-amber-400/5 p-4"
        >
          <h2 className="font-medium">{item.request.toolName}</h2>
          <p className="text-sm">{item.request.reason}</p>
          <p className="break-all text-xs text-text-secondary">
            {item.request.targetValue}
          </p>
          <pre className="max-h-56 overflow-auto whitespace-pre-wrap break-all text-xs">
            {item.request.argumentsPreview}
          </pre>
          <div className="flex flex-wrap gap-2">
            {item.request.choices?.length ? item.request.choices.map((choice, index) => (
              <button key={index} className={remoteButton} onClick={() => void act('approvals.respond', { requestId: item.request.id, decision: `select_option:${index}` })}>{choice}</button>
            )) : <button
              className={remoteButton}
              onClick={() =>
                void act("approvals.respond", {
                  requestId: item.request.id,
                  decision: "allow_once",
                })
              }
            >
              {t("remote.allowOnce")}
            </button>}
            <button
              className={remoteButton}
              onClick={() =>
                void act("approvals.respond", {
                  requestId: item.request.id,
                  decision: "deny",
                })
              }
            >
              {t("remote.deny")}
            </button>
          </div>
        </section>
      ))}
      {interactions.map((item) => (
        <form
          key={item.interactionId}
          className="space-y-3 rounded-2xl border border-accent/30 p-4"
          onSubmit={(event) => {
            event.preventDefault();
            void act("interactions.respond", {
              input: {
                interactionId: item.interactionId,
                resumeToken: item.resumeToken,
                answers: answers[item.interactionId] || {},
              },
            });
          }}
        >
          <h2 className="font-medium">{item.title}</h2>
          {item.description && (
            <p className="text-sm text-text-secondary">{item.description}</p>
          )}
          {item.questions.map((question) => (
            <fieldset key={question.id} className="space-y-2">
              <legend className="mb-2 text-sm">{question.question}</legend>
              {question.options?.map((option) => (
                <label
                  key={option.label}
                  className="flex items-start gap-2 text-sm"
                >
                  <input
                    type={
                      question.type === "multi_choice" ? "checkbox" : "radio"
                    }
                    name={question.id}
                    checked={
                      answers[item.interactionId]?.[question.id]?.includes(
                        option.label,
                      ) || false
                    }
                    onChange={(event) =>
                      setAnswers((current) => {
                        const existing =
                          current[item.interactionId]?.[question.id] || [];
                        const next =
                          question.type === "multi_choice"
                            ? event.target.checked
                              ? [...existing, option.label]
                              : existing.filter(
                                  (value) => value !== option.label,
                                )
                            : [option.label];
                        return {
                          ...current,
                          [item.interactionId]: {
                            ...current[item.interactionId],
                            [question.id]: next,
                          },
                        };
                      })
                    }
                  />
                  <span>
                    {option.label}
                    {option.description && (
                      <small className="block text-text-secondary">
                        {option.description}
                      </small>
                    )}
                  </span>
                </label>
              ))}
              <textarea
                aria-label={question.question}
                className={remoteField}
                rows={question.type === "long" ? 3 : 1}
                placeholder={t("remote.customAnswer")}
                value={(answers[item.interactionId]?.[question.id] || [])
                  .filter(
                    (value) =>
                      !question.options?.some(
                        (option) => option.label === value,
                      ),
                  )
                  .join("\n")}
                onChange={(event) => {
                  const custom = event.target.value;
                  setAnswers((current) => {
                    const selected =
                      question.type === "multi_choice"
                        ? (current[item.interactionId]?.[question.id] || []).filter(
                            (value) =>
                              question.options?.some(
                                (option) => option.label === value,
                              ),
                          )
                        : [];
                    return {
                      ...current,
                      [item.interactionId]: {
                        ...current[item.interactionId],
                        [question.id]: [
                          ...new Set([...selected, ...(custom ? [custom] : [])]),
                        ],
                      },
                    };
                  });
                }}
              />
            </fieldset>
          ))}
          <button className={remoteButton} type="submit">
            {t("remote.submit")}
          </button>
        </form>
      ))}
      {!messages.length && !timeline.length && !loading && (
        <div className={mobile ? "native-chat-empty" : "py-8 text-center text-sm leading-7 text-text-secondary"}>
          {emptyState ?? t("remote.chatEmpty")}
        </div>
      )}
      </div>
      <form
        data-testid="remote-composer"
        className={mobile ? "remote-composer native-composer" : "remote-composer sticky bottom-3 z-20 mt-auto rounded-2xl border border-border p-3"}
        onSubmit={(event) => {
          event.preventDefault();
          followOutput.current = true; void send();
        }}
      >
        {followups && <div className="remote-followups" data-testid="followup-suggestions" aria-label={t('chat.followupSuggestions')}>{followups.items.map(item => <button key={item.prompt} type="button" title={item.prompt} onClick={() => { setDraft(item.prompt); draftRef.current = item.prompt; followOutput.current = true; void send(item.prompt); }}>{item.label}<ArrowUp size={13}/></button>)}</div>}
        <textarea ref={composerInput} rows={mobile ? 1 : undefined}
          aria-label={t("remote.message")}
          className={mobile ? "remote-composer-input native-composer-input" : "remote-composer-input max-h-52 min-h-24 w-full resize-y rounded-lg p-2 text-sm leading-6 outline-none"}
          maxLength={64000}
          placeholder={t("remote.messageHint")}
          value={draft}
          onChange={(event) => { draftRef.current = event.target.value; setDraft(event.target.value); }}
          aria-description={followups?.items[0] ? t('chat.followupTabHint', { prompt: followups.items[0].prompt }) : undefined}
          onKeyDown={event => {
            if (event.key === 'Tab' && !event.shiftKey && !event.ctrlKey && !event.altKey && !event.metaKey && !event.nativeEvent.isComposing && followups?.items[0]) {
              event.preventDefault(); draftRef.current = followups.items[0].prompt; setDraft(followups.items[0].prompt);
            }
          }}
        />
        {client.composerOptionsAvailable !== false && (!mobile || optionsOpen) && <div className="mb-3"><RemoteComposerOptions attachments={attachments} onAttachments={setAttachments} settings={composerSettings} onSettings={setComposerSettings} disabled={sending || running} onBusy={setLoadingAttachments} extendedModes={client.extendedModesAvailable !== false} imagesOnly={client.imagesOnly} /></div>}
        <div className={mobile ? 'native-composer-actions' : 'flex flex-wrap items-center justify-between gap-2'}>
          {client.approvalModeAvailable && <label className="mobile-approval-chip" data-mode={composerSettings.toolApprovalMode ?? 'ask'} title={t('settings.toolApproval')}>
            {composerSettings.toolApprovalMode === 'allow_all' ? <ShieldCheck size={15}/> : <Shield size={15}/>}
            <select aria-label={t('settings.toolApproval')} value={composerSettings.toolApprovalMode ?? 'ask'} disabled={sending || running} onChange={event => setComposerSettings(current => ({ ...current, toolApprovalMode: event.target.value as RemoteComposerSettings['toolApprovalMode'] }))}>
              <option value="ask">{t('settings.toolApprovalAsk')}</option><option value="allow_all">{t('settings.toolApprovalAllowAll')}</option><option value="deny_all">{t('settings.toolApprovalDenyAll')}</option>
            </select>
          </label>}
          {mobile && <><button type="button" className="mobile-icon-button" aria-label={client.composerOptionsAvailable === false ? t('remote.mobileFiles') : t('remote.addAttachments')} onClick={() => client.composerOptionsAvailable === false ? mobile.onFiles() : setOptionsOpen(value => !value)}>{client.composerOptionsAvailable === false ? <FolderOpen size={20}/> : <Plus size={20}/>}</button><button type="button" className="mobile-model-chip" onClick={() => client.modelPickerAvailable === false ? mobile.onConfigure() : setModelOpen(true)}>{modelSelection?.model || connections.find(item => item.id === connectionId)?.model || t('remote.model')}<ChevronDown size={13}/></button><span className="native-composer-spacer"/></>}
          {client.nativeSpeech && <NativeDictationInput key={conversationId} speech={client.nativeSpeech} disabled={sending} onBusy={setDictating} onEvent={event => {
            const projected = applyVoiceDictationEvent(draftRef.current, voiceDraft.current, event);
            voiceDraft.current = projected.session; draftRef.current = projected.draft; setDraft(projected.draft);
          }}/>}
          {!client.nativeSpeech && client.voiceAvailable !== false && <RemoteVoiceInput key={conversationId} client={client} disabled={sending} onBusy={setDictating} onEvent={event => {
            const projected = applyVoiceDictationEvent(draftRef.current, voiceDraft.current, event);
            voiceDraft.current = projected.session; draftRef.current = projected.draft; setDraft(projected.draft);
          }} />}
          <button
            type={running ? "button" : "submit"}
            className={mobile ? `native-send composer-submit ${running ? 'is-stopping' : ''}` : `${remoteButton} composer-submit !border-accent !bg-accent text-white`}
            aria-label={t(running ? "remote.stop" : "remote.send")}
            onClick={running ? () => void act("chat.stop", { conversationId }) : undefined}
            disabled={!running && (sending || dictating || loadingAttachments || !connectionId || (!draft.trim() && !attachments.length))}
          >
            <span className="composer-submit-icon" key={running ? 'stop' : sending ? 'sending' : 'send'}>{running ? <CircleStop size={20}/> : sending ? <Loader2 size={18} className="animate-spin"/> : mobile ? <ArrowUp size={20}/> : <Send size={16}/>}</span>
            <span className={mobile ? "sr-only" : undefined}>{t(running ? "remote.stop" : "remote.send")}</span>
          </button>
        </div>
      </form>
      {mobile && <p className="native-chat-caption">{mobile.caption}</p>}
    </main>
  );
}

function RemoteChatMessage({ message, onMore, onMoreThinking, onSpeak, speaking }: { message: ChatMessage; onMore: () => void; onMoreThinking: () => void; onSpeak?: () => void; speaking?: boolean }) {
  const { t } = useTranslation();
  const answer = <article className={`remote-message ${message.role === 'user' ? 'remote-message-user' : 'remote-answer'}`}>
    <p className="mb-3 text-xs font-semibold text-text-tertiary">{message.role === 'user' ? t('remote.you') : 'Nexa'}</p>
    {!!message.thinking && <details className="remote-thinking" data-testid="remote-persisted-thinking"><summary>{t('remote.thinking')}</summary><div className="max-h-80 overflow-auto"><StreamingMarkdown content={message.thinking} isStreaming={false} reduceMotion/>{[...message.thinking].length < (message.thinkingTotalChars ?? 0) && <button className={remoteButton} onClick={onMoreThinking}>{t('remote.moreText')}</button>}</div></details>}
    {message.role === 'user' ? <div className="whitespace-pre-wrap break-words text-sm leading-7">{message.content}</div>
      : <StreamingMarkdown content={stripFollowupBlocks(message.content)} isStreaming={false} reduceMotion />}
    {[...message.content].length < message.totalChars && <button className={`${remoteButton} mt-3`} onClick={onMore}>
      <ArrowDown size={14} />{t('remote.moreText')}
    </button>}
    {message.role === 'assistant' && onSpeak && <button className="mobile-read-aloud" aria-label={t(speaking ? 'remote.stopReading' : 'remote.readAloud')} onClick={onSpeak}>{speaking ? <CircleStop size={16}/> : <Volume2 size={16}/>}</button>}
  </article>;
  return message.intermediate ? <details className="remote-activity" data-testid="remote-persisted-activity"><summary className="cursor-pointer text-xs text-text-secondary">{t('remote.activity')}</summary><div className="mt-4">{answer}</div></details> : answer;
}
