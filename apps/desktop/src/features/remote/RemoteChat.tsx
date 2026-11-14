import { modelTokensPerSecond } from '../../lib/contextUsage';
import { useGenerationMetrics } from '../../lib/useGenerationMetrics';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ListChecks, Trash2, ArrowUp, ChevronDown, CircleStop, CornerDownRight, FolderOpen, Loader2, Plus, Send, Shield, ShieldCheck, SquarePen } from "lucide-react";
import { MobileConversationRow } from './MobileConversationRow';
import { MobileSchedules } from './MobileSchedules';
import { CalendarClock } from 'lucide-react';
import { AnimatePresence } from 'framer-motion';
import { MobileProjectHistory } from './MobileProjectHistory';

import { MobileChatHeader, MobileDialog, type MobileChatPresentation } from './MobileChatShell';
import { useTranslation } from "../../i18n";
import type { RemoteChatClient } from "./remoteClient";
import { remoteButton, remoteField } from "./remoteUi";
import { ConnectionModelPicker } from '../models/ConnectionModelPicker';
import type { TurnModelSelection } from '../models/modelChoices';
import { RemoteVoiceInput } from './RemoteVoiceInput';
import { useHoldToDictate, type DictationControls } from '../voice/useHoldToDictate';
import { NativeDictationInput } from './NativeDictationInput';
import { nativeSpeechErrorKey } from './nativeSpeechErrors';
import { NexaCombobox } from '../../components/ui/overlay/Combobox';
import { finalAnswerToSpeechText } from '../../lib/autoSpeech';
import { defaultFollowupSuggestions, latestFollowupSuggestions } from '../../lib/followupSuggestions';
import { applyVoiceDictationEvent, type VoiceDraftSession } from '../voice/voiceDraftProjection';
import { RemoteComposerOptions, defaultComposerSettings, type RemoteComposerSettings } from './RemoteComposerOptions';
import type { AgentRunEvent, ContextUsageBreakdown, ImageAttachment } from '../../types/conversation';
import { enqueueStreamRunEvent, takeNextStreamRunEvent, takeAuthoritativeRunEventSuffix, type StreamEventOrderingState } from '../../lib/streaming/ordering';
import type { RemoteTimelineItem } from './RemoteRunTimeline';
import { RemoteTranscript, type ChatMessage } from './RemoteTranscript';
import { completeReplayText, mergeRemoteHistory } from './remoteHistory';
import { useMobileChatProjects } from './useMobileChatProjects';
import { recoveredDeltaSuffix, utf8ByteLength } from '../../lib/streaming/blockTrace';

// Durable fragments are at most 64 KiB. Retain enough UTF-16 units to validate
// that entire overlap even after the independently bounded visible prefix.
const LIVE_TEXT_PREFIX_CHARS = 128_000;
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
interface MessagePage {
  messages: ChatMessage[];
  beforeOrder: number | null;
}
type RunEvent = AgentRunEvent;
interface RunPage {
  run: { id: string; status: string; phase?: string; userMessageId?: string; finalMessageId?: string | null; errorMessage?: string | null } | null;
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
  const { projectView, drawerConversations } = useMobileChatProjects(mobile?.projectStore, client, drawerOpen, conversations, cursor);
  const pendingProjectMove = useRef<{ conversationId: string; projectId: string } | null>(null);
  const [conversationId, setConversationId] = useState(
    () =>
      localStorage.getItem(
        `nexa.remote.chat.${client.paired.manifest.serverId}`,
      ) || "",
  );
  const selectedConversation = useRef(conversationId);
  const loadAttachment = useCallback((messageId: string, index: number) => client.rpc<ImageAttachment>('chat.attachment', {
    conversationId, messageId, index,
  }), [client, conversationId]);
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
  const [schedulesOpen, setSchedulesOpen] = useState(false);
  const [restoringId, setRestoringId] = useState<string | null>(null);
  const [archivesOpen, setArchivesOpen] = useState(false);
  const [deleteArchivedIds, setDeleteArchivedIds] = useState<string[] | null>(null);
  const [manageTarget, setManageTarget] = useState<{ item: Conversation; action: "archive" | "delete" } | null>(null);
  const destination = mobile?.notificationTarget;
  const onNotificationHandled = mobile?.onNotificationHandled;
  useEffect(() => {
    if (!destination) return;
    selectConversation(destination.conversationId);
    pendingSend.current = null;
    followOutput.current = true;
    setDrawerOpen(false); setModelOpen(false); setOptionsOpen(false);
    setManageTarget(null); setDeleteArchivedIds(null); setSchedulesOpen(!!destination.schedules);
    onNotificationHandled?.();
  }, [destination, onNotificationHandled]);
  const [managing, setManaging] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [timeline, setTimeline] = useState<RemoteTimelineItem[]>([]);
  const [timelineRunId, setTimelineRunId] = useState<string | null>(null);
  const [runUserId, setRunUserId] = useState<string | null>(null);
  const [finalMessageId, setFinalMessageId] = useState<string | null>(null);
  const markReplyRead = mobile?.onReplyRead;
  // Replay may reparent the canonical answer under a timeline after the
  // final ID is known. Observe the currently mounted article in either layout.
  const [renderedReply, setRenderedReply] = useState<HTMLElement | null>(null);
  useEffect(() => {
    if (!markReplyRead || !conversationId || !timelineRunId || !finalMessageId || drawerOpen) return;
    const root = contentScroll.current;
    if (!root || renderedReply?.dataset.messageId !== finalMessageId || !root.contains(renderedReply)) return;
    let visible = false;
    const markVisible = () => {
      if (visible && document.visibilityState === 'visible' && !document.querySelector('dialog[open]')) {
        void markReplyRead(conversationId, timelineRunId).catch(() => {});
      }
    };
    const observer = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      if (visible) markVisible();
    }, { root });
    observer.observe(renderedReply);
    window.addEventListener('focus', markVisible);
    document.addEventListener('visibilitychange', markVisible);
    document.addEventListener('close', markVisible, true);
    return () => {
      observer.disconnect();
      window.removeEventListener('focus', markVisible);
      document.removeEventListener('visibilitychange', markVisible);
      document.removeEventListener('close', markVisible, true);
    };
  }, [markReplyRead, conversationId, timelineRunId, finalMessageId, renderedReply, drawerOpen]);
  const [runStatus, setRunStatus] = useState('');
  const generationScope = JSON.stringify([client.paired.manifest.serverId, conversationId, connectionId,
    modelSelection?.model ?? connections.find(item => item.id === connectionId)?.model]);
  const generationScopeRef = useRef(generationScope);
  generationScopeRef.current = generationScope;
  const [observedGeneration, setGeneration] = useState<ContextUsageBreakdown['generation']>();
  const generation = useGenerationMetrics(
    generationScope,
    observedGeneration,
  );
  const generationTps = modelTokensPerSecond(generation);
  const [syncingHistory, setSyncingHistory] = useState(false);
  const [progress, setProgress] = useState("");
  const [running, setRunning] = useState(false);
  const [pendingSteers, setPendingSteers] = useState<{ id: string; runId: string; message: string }[]>([]);
  const [dismissedFollowup, setDismissedFollowup] = useState('');
  const canSteer = running && !!timelineRunId && runStatus !== 'cancelling' &&
    client.paired.manifest.capabilities?.includes('chat-steering-v1');
  const submittingSteer = canSteer && !!draft.trim() && !attachments.length;
  const dictationControls = useRef<DictationControls>(null);
  const heldControls = useRef<DictationControls | null>(null);
  const hasDictation = client.nativeSpeech ? !!client.nativeSpeech.finish : client.voiceAvailable !== false;
  const holdHandlers = useHoldToDictate({
    enabled: hasDictation && !sending && !dictating,
    onStart: () => { heldControls.current = dictationControls.current; heldControls.current?.start(); },
    onFinish: () => { heldControls.current?.finish(); heldControls.current = null; },
    onCancel: () => { heldControls.current?.cancel(); heldControls.current = null; },
  });
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
  const speak = useCallback(async (message: ChatMessage) => {
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
  }, [client, conversationId, locale, speakingId, t]);
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
  const consumedInitialDraft = useRef('');
  useEffect(() => {
    if (!initialDraft) consumedInitialDraft.current = '';
    if (initialDraft && consumedInitialDraft.current !== initialDraft) {
      consumedInitialDraft.current = initialDraft;
      setDraft(current => [current, initialDraft].filter(Boolean).join('\n\n'));
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
    let runGenerationScope: string | null = null;
    let finalId: string | null = null;
    let status = '';
    let textReplayAttempts = 0;
    const ordering: StreamEventOrderingState = {
      _orderedRunId: null, _lastEventSeq: 0, _pendingRunEvents: new Map(),
    };
    const blocks = new Map<string, { text: string; bytes: number; tail: string; truncated?: boolean }>();
    const items = new Map<string, RemoteTimelineItem>();
    const encoder = new TextEncoder();
    let paint: ReturnType<typeof setTimeout> | null = null;
    const retired = new Set<string>();
    const terminal = new Set<string>();
    setGeneration(undefined); setTimeline([]); setTimelineRunId(null); setRunUserId(null); setFinalMessageId(null); setRunStatus('');
    setProgress(''); setRunning(false); setMessages([]); setPendingSteers([]); setDismissedFollowup('');
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
      runGenerationScope = generationScopeRef.current;
      ordering._orderedRunId = id; ordering._lastEventSeq = 0; ordering._pendingRunEvents.clear();
      blocks.clear(); items.clear(); readNeeded = true;
      setTimeline([]); setTimelineRunId(id); setRunUserId(null); setFinalMessageId(null); setProgress('');
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
      if (event.displayKind === 'steering') {
        // The accepted event can race the RPC reply. History owns the full
        // text and its position; the event label can be truncated to 280 chars.
        setPendingSteers(current => {
          const index = current.findIndex(item => {
            if (item.runId !== event.runId) return false;
            const compact = item.message.trim().replace(/\s+/g, ' ');
            return event.label === item.message || event.label === compact ||
              (event.label.endsWith('…') && compact.startsWith(event.label.slice(0, -1)));
          });
          return index < 0 ? current : current.filter((_, position) => position !== index);
        });
        readNeeded = true;
      }
      if (event.visibility === 'user' && event.label) setProgress(event.label);
      const payload = (event.payload ?? {}) as Record<string, any>;
      if (event.kind === 'usageUpdated' || event.kind === 'done') {
        const metrics = (payload.contextBreakdown ?? payload.usageTotal?.contextBreakdown)?.generation;
        if (runGenerationScope === generationScopeRef.current && modelTokensPerSecond(metrics) != null) setGeneration(metrics);
      }
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
          const snapshot = event.kind === 'outputSnapshot';
          const combined = snapshot ? text : previous.text + text;
          const next = {
            text: combined.slice(0, LIVE_TEXT_PREFIX_CHARS),
            bytes: (snapshot ? 0 : previous.bytes) + encoder.encode(text).byteLength,
            tail: replayTail(snapshot ? text : previous.tail + text),
            truncated: combined.length > LIVE_TEXT_PREFIX_CHARS || (!snapshot && Boolean(previous.truncated)),
          };
          blocks.set(id, next);
          const priorItem = items.get(id);
          items.set(id, { id, kind: payload.channel, sequence: priorItem?.sequence ?? event.eventSeq, text: next.text, truncated: next.truncated });
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
      while (items.size > 128) {
        const oldest = items.keys().next().value!;
        items.delete(oldest);
        // A long tool loop can generate thousands of distinct text blocks.
        // Keep the offset for a later continuation without retaining every body.
        const block = blocks.get(oldest);
        if (block) blocks.set(oldest, {bytes: block.bytes, text: '…', tail: '', truncated: true});
      }
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
          setRunUserId(page.run.userMessageId ?? null);
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
    // Live events remain immediate. A quiet or completed conversation needs
    // only an occasional missed-event check, and an ongoing replay owns its cursor.
    let nextIdleCheck = performance.now() + 30_000;
    const timer = setInterval(() => {
      if (syncing || client.state.phase !== 'connected' || document.visibilityState === 'hidden') return;
      // The initial history read and live events can change the run after the
      // timer was installed. Decide from the current run at each heartbeat.
      const settled = !readNeeded && (!runId || terminal.has(runId));
      if (settled && performance.now() < nextIdleCheck) return;
      nextIdleCheck = performance.now() + 30_000;
      void sync();
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
    pendingProjectMove.current = null;
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
    setRestoringId(id); setManaging(true); setError('');
    try { await client.rpc('chat.restore', { conversationId: id }); await Promise.all([list(), listArchived()]); }
    catch (error) { setRestoringId(null); errorText(error); }
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
    let deleted = 0;
    try {
      for (const id of deleteArchivedIds) {
        const count = await client.rpc<number>('chat.deleteArchived', { conversationId: id });
        deleted++;
        if (count > 0 && selectedConversation.current === id) { selectConversation(''); pendingSend.current = null; }
      }
      setDeleteArchivedIds(null);
    } catch (error) { setDeleteArchivedIds(deleteArchivedIds.slice(deleted)); errorText(error); }
    finally { await Promise.all([list(), listArchived()]).catch(errorText); setManaging(false); }
  }
  async function manageConversation() {
    if (!manageTarget || managing) return;
    listGeneration.current++; archiveGeneration.current++;
    setManaging(true); setError("");
    try {
      const method = manageTarget.action === 'delete' && client.archivedDeletionAvailable
        ? 'chat.deleteArchived' : `chat.${manageTarget.action}`;
      const count = await client.rpc<number>(method, { conversationId: manageTarget.item.id });
      if (count !== 0 && selectedConversation.current === manageTarget.item.id) {
        selectConversation(""); pendingSend.current = null;
        localStorage.removeItem(`nexa.remote.chat.${client.paired.manifest.serverId}`);
      }
      setManageTarget(null); await list(); if (archivesOpen) await listArchived();
    } catch (error) { errorText(error); }
    finally { setManaging(false); }
  }
  async function send(message = draft) {
    if (sendLock.current || dictating || loadingAttachments || (!message.trim() && !attachments.length)) return;
    if (running && (!canSteer || !message.trim() || attachments.length)) return;
    if (!running && !connectionId) return;
    const generation = scope.current;
    sendLock.current = true;
    setSending(true);
    setError("");
    try {
      if (running && timelineRunId) {
        const item = { id: crypto.randomUUID(), runId: timelineRunId, message: message.trim() };
        setPendingSteers(current => [...current, item]);
        try {
          await client.rpc('chat.steer', { conversationId, runId: item.runId, message: item.message });
          if (selectedConversation.current === conversationId) {
            setDraft(current => current === message ? '' : current);
            refreshRef.current();
          }
        } catch (error) {
          setPendingSteers(current => current.filter(entry => entry.id !== item.id));
          throw error;
        }
        return;
      }
      let id = conversationId;
      if (!id) {
        const projectId = projectView?.selectedId;
        const conversation = await client.rpc<Conversation>("chat.create", {
          connectionId,
          modelSelection,
        });
        id = conversation.id;
        if (projectId) pendingProjectMove.current = { conversationId: id, projectId };
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
      // The chat now has a durable ID. On a failed move, a second send
      // retries the same association rather than creating a second chat.
      const destination = pendingProjectMove.current;
      if (destination?.conversationId === id) {
        if (projectView && !projectView.memberships[id] && projectView.projects.some(project => project.id === destination.projectId)) {
          await projectView.move(id, destination.projectId);
        }
        pendingProjectMove.current = null;
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
  const more = useCallback(async (message: ChatMessage, channel: 'content' | 'thinking' = 'content') => {
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
      if (valid()) setError(error instanceof Error ? error.message : String(error));
    }
  }, [client, conversationId]);
  const lastMessage = messages[messages.length - 1];
  const suggestions = useMemo(() => {
    if (running || (runStatus && runStatus !== 'completed') || lastMessage?.role !== 'assistant' ||
      lastMessage.intermediate || !lastMessage.content.trim() || (finalMessageId && lastMessage.id !== finalMessageId)) return null;
    const defaults = defaultFollowupSuggestions(t);
    // A page can end inside an example fence. Only its complete Markdown
    // document can authorize model-supplied options; offer local choices now.
    if ([...lastMessage.content].length < lastMessage.totalChars) {
      // The first history page is bounded, but live/durable replay often
      // already owns the entire final block. Parse it only when both agree.
      const last = timeline[timeline.length - 1];
      const replay = completeReplayText(lastMessage.content, lastMessage.totalChars, last?.kind === 'answer' ? last.text : undefined);
      if (replay) {
        return latestFollowupSuggestions([{...lastMessage, content:replay}], defaults);
      }
      return { key: lastMessage.id, items: defaults };
    }
    return latestFollowupSuggestions([lastMessage], defaults);
  }, [running, runStatus, lastMessage, finalMessageId, timeline, t]);
  const followups = !sending && !dictating && !draft && !attachments.length && suggestions?.key !== dismissedFollowup ? suggestions : null;
  const ghostPrompt = followups?.items[0]?.prompt;
  const draftProject = !conversationId && projectView?.projects.find(project => project.id === projectView.selectedId);
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
        <MobileDialog open={drawerOpen} side title={t('remote.conversation')} onClose={() => setDrawerOpen(false)}><button className="mobile-drawer-new" disabled={sending || !connectionId} onClick={() => { setDrawerOpen(false); void create(); }}><SquarePen size={18}/>{t('remote.newChat')}</button><MobileProjectHistory projects={projectView} conversations={drawerConversations} activeId={conversationId}
          archiveId={manageTarget?.action === 'archive' ? manageTarget.item.id : undefined}
          onArchive={client.conversationManagementAvailable ? item => setManageTarget({ item, action:'archive' }) : undefined}
          onOpen={item => { selectConversation(item.id); pendingSend.current = null; followOutput.current = true; setDrawerOpen(false); }}
          hasMore={!!cursor} onMore={() => void list(cursor).catch(errorText)}/>
        {client.conversationManagementAvailable && <>
          <div className="mobile-archive-heading">
            <button className="mobile-add-connection" aria-expanded={archivesOpen} onClick={() => {
              setArchivesOpen(!archivesOpen);
              if (!archivesOpen) void listArchived().catch(errorText);
            }}>{t('chat.archivedConversations')}</button>
            {archivesOpen && client.archivedDeletionAvailable && archived.length > 0 &&
              <button className="mobile-icon-button" disabled={managing} aria-label={t('chat.deleteAll')} title={t('chat.deleteAll')} onClick={() => void prepareDeleteArchived()}><Trash2 size={17}/></button>}
          </div>
          {archivesOpen && <>{error && <p role="alert" className="mobile-error">{error}</p>}
            <nav className="mobile-history mobile-archive-history"><AnimatePresence initial={false} custom={restoringId}>{archived.map(item => {
              const title = item.title || t('remote.newChat');
              return <MobileConversationRow key={item.id} id={item.id} title={title} disabled={managing}
                restoring={restoringId === item.id && managing}
                preview={manageTarget?.item.id === item.id && manageTarget.action === 'delete'}
                onRestore={() => void restoreConversation(item.id)}
                onDelete={client.archivedDeletionAvailable ? () => { setRestoringId(null); setManageTarget({ item, action: 'delete' }); } : undefined}>
                <span className="min-w-0 flex-1 truncate">{title}</span>
              </MobileConversationRow>;
            })}</AnimatePresence></nav>
            {archivedCursor && <button onClick={() => void listArchived(archivedCursor).catch(errorText)}>{t('remote.moreChats')}</button>}
          </>}
        </>}
        {client.paired.manifest.capabilities?.includes('chat-schedules-v1') && <button className="mobile-drawer-new" onClick={() => { setDrawerOpen(false); setSchedulesOpen(true); }}><CalendarClock size={17}/>{t('chat.scheduledTasks')}</button>}
        <div className="mobile-drawer-footer" onClick={() => setDrawerOpen(false)}>{mobile.drawerFooter}</div></MobileDialog>
        {schedulesOpen && <MobileSchedules client={client} conversationId={conversationId} onClose={()=>setSchedulesOpen(false)} ensureChat={async () => {
          const item = await client.rpc<Conversation>('chat.create', {connectionId,modelSelection});
          selectConversation(item.id); setConversations(current => [item,...current.filter(chat=>chat.id!==item.id)]);
          if (projectView?.selectedId) await projectView.move(item.id, projectView.selectedId);
          return item.id;
        }}/>}
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
        <RemoteTranscript messages={messages} timeline={timeline} running={running} finalMessageId={finalMessageId} runUserId={runUserId} runId={timelineRunId} answerRef={markReplyRead ? setRenderedReply : undefined}
          more={more} speak={client.nativeSpeech ? speak : undefined} speakingId={speakingId} loadAttachment={loadAttachment}/>
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
          {generationTps != null && <span data-testid="chat-generation-tps-summary" className="shrink-0 text-xs tabular-nums" title={`${t(generation?.externalTurn ? 'chat.externalTurnSpeed' : 'chat.generationSpeed')}: ${t(generation?.externalTurn ? 'chat.externalTurnSpeedHint' : 'chat.generationSpeedHint')}`}>
            {generationTps.toFixed(1)} tok/s
          </span>}
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
        {pendingSteers.length > 0 && <div className="max-h-24 overflow-y-auto" aria-live="polite">{pendingSteers.map(item =>
          <div className="flex min-w-0 items-center gap-2 px-2 py-1 text-xs text-text-secondary" key={item.id}>
            <CornerDownRight size={14} aria-hidden="true"/><span className="min-w-0 flex-1 truncate" title={item.message}>{item.message}</span>
            {running && timelineRunId === item.runId ? <span>{t('chat.subagentInputQueued')}</span> :
              <button type="button" className={remoteButton} onClick={() => {
                setDraft(current => [current, item.message].filter(Boolean).join('\n'));
                setPendingSteers(current => current.filter(entry => entry.id !== item.id));
                composerInput.current?.focus();
              }}>{t('chat.editDraft')}</button>}
          </div>
        )}</div>}
        {followups && <div className="remote-followups" data-testid="followup-suggestions" aria-label={t('chat.followupSuggestions')}>{followups.items.map(item => <button key={item.prompt} type="button" title={item.prompt} onClick={() => { setDraft(item.prompt); draftRef.current = item.prompt; followOutput.current = true; void send(item.prompt); }}>{item.label}<ArrowUp size={13}/></button>)}</div>}
        <div className="flex min-w-0 items-start">
        <textarea {...holdHandlers} ref={composerInput} rows={mobile ? 1 : undefined}
          aria-label={t("remote.message")}
          className={mobile ? "remote-composer-input native-composer-input" : "remote-composer-input max-h-52 min-h-24 w-full resize-y rounded-lg p-2 text-sm leading-6 outline-none"}
          maxLength={64000}
          placeholder={ghostPrompt || t(canSteer ? 'chat.steeringMessage' : 'remote.messageHint')}
          value={draft}
          onChange={(event) => { draftRef.current = event.target.value; setDraft(event.target.value); }}
          aria-description={[hasDictation && t('voice.holdComposerToDictate'), followups?.items[0] && t('chat.followupTabHint', { prompt: followups.items[0].prompt })].filter(Boolean).join(' ') || undefined}
          onKeyDown={event => {
            if (event.key === 'Escape' && followups && !event.nativeEvent.isComposing) {
              event.preventDefault(); setDismissedFollowup(followups.key);
            }
            if (event.key === 'Tab' && !event.shiftKey && !event.ctrlKey && !event.altKey && !event.metaKey && !event.nativeEvent.isComposing && followups?.items[0]) {
              event.preventDefault(); draftRef.current = followups.items[0].prompt; setDraft(followups.items[0].prompt);
            }
          }}
        />
        {ghostPrompt && <button type="button" aria-label={t('chat.editDraft')} title={ghostPrompt}
          className="inline-flex size-11 shrink-0 items-center justify-center rounded-full text-text-tertiary"
          onClick={() => { draftRef.current = ghostPrompt; setDraft(ghostPrompt); composerInput.current?.focus(); }}>
          <SquarePen size={16}/>
        </button>}
        </div>
        {client.composerOptionsAvailable !== false && (!mobile || optionsOpen) && <div className="mb-3"><RemoteComposerOptions showExecutionMode={!mobile} attachments={attachments} onAttachments={setAttachments} settings={composerSettings} onSettings={setComposerSettings} disabled={sending || running} onBusy={setLoadingAttachments} extendedModes={client.extendedModesAvailable !== false} imagesOnly={client.imagesOnly} /></div>}
        <div className={mobile ? 'native-composer-actions' : 'flex flex-wrap items-center justify-between gap-2'}>
          {mobile && client.composerOptionsAvailable !== false && <button type="button" className="mobile-plan-chip" aria-label={t('remote.modePlan')} title={t('remote.modePlan')} aria-pressed={composerSettings.executionMode === 'plan'} disabled={sending || running} onClick={() => setComposerSettings(current => ({ ...current, executionMode: current.executionMode === 'plan' ? 'normal' : 'plan' }))}><ListChecks size={16}/></button>}
          {client.approvalModeAvailable && <label className="mobile-approval-chip" data-mode={composerSettings.toolApprovalMode ?? 'ask'} title={t('settings.toolApproval')}>
            {composerSettings.toolApprovalMode === 'allow_all' ? <ShieldCheck size={15}/> : <Shield size={15}/>}
            <select aria-label={t('settings.toolApproval')} value={composerSettings.toolApprovalMode ?? 'ask'} disabled={sending || running} onChange={event => setComposerSettings(current => ({ ...current, toolApprovalMode: event.target.value as RemoteComposerSettings['toolApprovalMode'] }))}>
              <option value="ask">{t('settings.toolApprovalAsk')}</option><option value="allow_all">{t('settings.toolApprovalAllowAll')}</option><option value="deny_all">{t('settings.toolApprovalDenyAll')}</option>
            </select>
          </label>}
          {mobile && <><button type="button" className="mobile-icon-button" aria-label={client.composerOptionsAvailable === false ? t('remote.mobileFiles') : t('remote.addAttachments')} onClick={() => client.composerOptionsAvailable === false ? mobile.onFiles() : setOptionsOpen(value => !value)}>{client.composerOptionsAvailable === false ? <FolderOpen size={20}/> : <Plus size={20}/>}</button><button type="button" className="mobile-model-chip" onClick={() => client.modelPickerAvailable === false ? mobile.onConfigure() : setModelOpen(true)}>{modelSelection?.model || connections.find(item => item.id === connectionId)?.model || t('remote.model')}<ChevronDown size={13}/></button><span className="native-composer-spacer"/></>}
          {client.nativeSpeech && <NativeDictationInput controls={dictationControls} key={conversationId} speech={client.nativeSpeech} disabled={sending} onBusy={setDictating} onEvent={event => {
            const projected = applyVoiceDictationEvent(draftRef.current, voiceDraft.current, event);
            voiceDraft.current = projected.session; draftRef.current = projected.draft; setDraft(projected.draft);
          }}/>}
          {!client.nativeSpeech && client.voiceAvailable !== false && <RemoteVoiceInput controls={dictationControls} key={conversationId} client={client} disabled={sending} onBusy={setDictating} onEvent={event => {
            const projected = applyVoiceDictationEvent(draftRef.current, voiceDraft.current, event);
            voiceDraft.current = projected.session; draftRef.current = projected.draft; setDraft(projected.draft);
          }} />}
          <button
            type={running && !submittingSteer ? "button" : "submit"}
            className={mobile ? `native-send composer-submit ${running && !submittingSteer ? 'is-stopping' : ''}` : `${remoteButton} composer-submit !border-accent !bg-accent text-white`}
            aria-label={t(submittingSteer ? "chat.steeringMessage" : running ? "remote.stop" : "remote.send")}
            onClick={running && !submittingSteer ? () => void act("chat.stop", { conversationId }) : undefined}
            disabled={submittingSteer ? sending || dictating : !running && (sending || dictating || loadingAttachments || !connectionId || (!draft.trim() && !attachments.length))}
          >
            <span className="composer-submit-icon" key={submittingSteer ? 'steer' : running ? 'stop' : sending ? 'sending' : 'send'}>{submittingSteer ? <CornerDownRight size={20}/> : running ? <CircleStop size={20}/> : sending ? <Loader2 size={18} className="animate-spin"/> : mobile ? <ArrowUp size={20}/> : <Send size={16}/>}</span>
            <span className={mobile ? "sr-only" : undefined}>{t(submittingSteer ? "chat.steeringMessage" : running ? "remote.stop" : "remote.send")}</span>
          </button>
        </div>
      </form>
      {mobile && <p className="native-chat-caption">{draftProject ? t('project.newConversationHint', { name: draftProject.name }) : mobile.caption}</p>}
    </main>
  );
}
