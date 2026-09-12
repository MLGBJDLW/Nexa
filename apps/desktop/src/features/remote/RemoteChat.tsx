import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowDown, CircleStop, Loader2, Plus, Send } from "lucide-react";
import { useTranslation } from "../../i18n";
import { RemoteClient } from "./remoteClient";
import { remoteButton, remoteField } from "./remoteUi";
import { ConnectionModelPicker } from '../models/ConnectionModelPicker';
import type { TurnModelSelection } from '../models/modelChoices';
import { RemoteVoiceInput } from './RemoteVoiceInput';
import { applyVoiceDictationEvent, type VoiceDraftSession } from '../voice/voiceDraftProjection';
import { StreamingMarkdown } from '../../components/chat/StreamingMarkdown';
import { RemoteComposerOptions, defaultComposerSettings, type RemoteComposerSettings } from './RemoteComposerOptions';
import type { AgentRunEvent, ImageAttachment } from '../../types/conversation';
import { enqueueStreamRunEvent, takeNextStreamRunEvent, takeAuthoritativeRunEventSuffix, type StreamEventOrderingState } from '../../lib/streaming/ordering';
import { RemoteRunTimeline, type RemoteTimelineItem } from './RemoteRunTimeline';

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
}: {
  client: RemoteClient;
  initialDraft: string;
  onDraftConsumed: () => void;
}) {
  const { t } = useTranslation();
  const [connections, setConnections] = useState<Connection[]>([]);
  const [connectionId, setConnectionId] = useState("");
  const [modelSelection, setModelSelection] = useState<TurnModelSelection | null>(null);
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [conversationId, setConversationId] = useState(
    () =>
      localStorage.getItem(
        `nexa.remote.chat.${client.paired.manifest.serverId}`,
      ) || "",
  );
  const selectedConversation = useRef(conversationId);
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
  const [composerSettings, setComposerSettings] = useState<RemoteComposerSettings>(defaultComposerSettings);
  const [loadingAttachments, setLoadingAttachments] = useState(false);
  const [sending, setSending] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [timeline, setTimeline] = useState<RemoteTimelineItem[]>([]);
  const [finalMessageId, setFinalMessageId] = useState<string | null>(null);
  const [runStatus, setRunStatus] = useState('');
  const [syncingHistory, setSyncingHistory] = useState(false);
  const [progress, setProgress] = useState("");
  const [running, setRunning] = useState(false);
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
      const result = await client.rpc<{
        items: Conversation[];
        nextCursor: string | null;
      }>("chat.list", { before });
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
    void Promise.all([client.rpc<Connection[]>("connections.list"), list()])
      .then(([items]) => {
        if (active) {
          setConnections(items);
          setConnectionId(
            (current) =>
              current ||
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
    return () => {
      active = false;
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
    const blocks = new Map<string, { text: string; bytes: number }>();
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
      setMessages(current => {
        const firstOrder = result.messages[0]?.sortOrder ?? Infinity;
        const older = current.filter(message => message.sortOrder < firstOrder);
        const latest = result.messages.map(message => {
          const expanded = current.find(old => old.id === message.id);
          return expanded && expanded.totalChars === message.totalChars && expanded.content.startsWith(message.content)
            ? expanded : message;
        });
        return [...older, ...latest];
      });
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
        const previous = blocks.get(id) ?? { text: '', bytes: 0 };
        if (event.kind === 'outputSnapshot' || previous.bytes === payload.offset) {
          const text = String(event.kind === 'outputSnapshot' ? payload.text ?? payload.content ?? '' : payload.delta ?? '');
          const next = event.kind === 'outputSnapshot'
            ? { text: text.slice(0, 128_000), bytes: encoder.encode(text).byteLength }
            : { text: (previous.text + text).slice(0, 128_000), bytes: previous.bytes + encoder.encode(text).byteLength };
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
        updateStatus(String(payload.status || event.status || (event.kind === 'done' ? 'completed' : 'failed')));
        readNeeded = true;
        if (event.kind === 'error' && typeof payload.message === 'string') setError(payload.message);
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
          if (page.run.errorMessage) setError(page.run.errorMessage);
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
    if (sending || !connectionId) return;
    const generation = scope.current;
    setSending(true);
    setError("");
    try {
      const conversation = await client.rpc<Conversation>("chat.create", {
        connectionId,
        modelSelection,
      });
      if (
        scope.current === generation &&
        selectedConversation.current === conversationId
      ) {
        selectConversation(conversation.id);
        pendingSend.current = null;
      }
      setConversations((current) => [
        conversation,
        ...current.filter((item) => item.id !== conversation.id),
      ]);
    } catch (error) {
      errorText(error);
    } finally {
      setSending(false);
    }
  }
  async function send() {
    if (sending || dictating || loadingAttachments || (!draft.trim() && !attachments.length) || !connectionId) return;
    const generation = scope.current;
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
        previous.message === draft &&
        previous.conversationId === id &&
        previous.connectionId === connectionId &&
        previous.attachments === attachments && previous.settings === composerSettings &&
        JSON.stringify(previous.modelSelection) === JSON.stringify(modelSelection)
          ? previous
          : {
              message: draft,
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
  async function more(message: ChatMessage) {
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
          offset: [...message.content].length,
        },
      );
      if (!valid()) return;
      setMessages((current) =>
        current.map((item) =>
          item.id === message.id && item.content === message.content
            ? { ...item, content: item.content + page.content }
            : item,
        ),
      );
    } catch (error) {
      if (valid()) errorText(error);
    }
  }
  return (
    <main
      className="mx-auto flex w-full max-w-3xl flex-1 flex-col gap-5 p-4 pb-8"
      data-testid="remote-chat"
    >
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
              {item.title}
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
      <select
        aria-label={t("remote.providerConnection")}
        className={remoteField}
        value={connectionId}
        onChange={(event) => { setConnectionId(event.target.value); setModelSelection(null); }}
      >
        {connections.map((item) => (
          <option key={item.id} value={item.id}>
            {item.name} · {item.model}
          </option>
        ))}
      </select>
      <ConnectionModelPicker connectionId={connectionId} defaultModel={connections.find(item => item.id === connectionId)?.model || ''}
        value={modelSelection} onChange={setModelSelection} load={client.models} label={t('remote.model')} disabled={sending} />
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
          <RemoteChatMessage key={message.id} message={message} onMore={() => void more(message)} />
        ))}
        <RemoteRunTimeline items={timeline} running={running} settled={Boolean(finalMessageId)} />
        {messages.filter(message => message.id === finalMessageId).map(message => (
          <RemoteChatMessage key={message.id} message={message} onMore={() => void more(message)} />
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
          {running && progress && <span className="min-w-0 flex-1 truncate text-text-tertiary">{progress}</span>}
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
          <div className="flex gap-2">
            <button
              className={remoteButton}
              onClick={() =>
                void act("approvals.respond", {
                  requestId: item.request.id,
                  decision: "allow_once",
                })
              }
            >
              {t("remote.allowOnce")}
            </button>
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
        <div className="py-8 text-center text-sm leading-7 text-text-secondary">
          {t("remote.chatEmpty")}
        </div>
      )}
      <form
        data-testid="remote-composer"
        className="remote-composer sticky bottom-3 z-20 mt-auto rounded-2xl border border-border p-3 shadow-lg"
        onSubmit={(event) => {
          event.preventDefault();
          void send();
        }}
      >
        <textarea
          aria-label={t("remote.message")}
          className="remote-composer-input max-h-52 min-h-24 w-full resize-y rounded-lg p-2 text-sm leading-6 outline-none"
          maxLength={64000}
          placeholder={t("remote.messageHint")}
          value={draft}
          onChange={(event) => { draftRef.current = event.target.value; setDraft(event.target.value); }}
        />
        <div className="mb-3"><RemoteComposerOptions attachments={attachments} onAttachments={setAttachments} settings={composerSettings} onSettings={setComposerSettings} disabled={sending || running} onBusy={setLoadingAttachments} /></div>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <RemoteVoiceInput key={conversationId} client={client} disabled={sending} onBusy={setDictating} onEvent={event => {
            const projected = applyVoiceDictationEvent(draftRef.current, voiceDraft.current, event);
            voiceDraft.current = projected.session; draftRef.current = projected.draft; setDraft(projected.draft);
          }} />
          {running && (
            <button
              type="button"
              className={`${remoteButton} text-danger`}
              onClick={() => void act("chat.stop", { conversationId })}
            >
              <CircleStop size={16} />
              {t("remote.stop")}
            </button>
          )}
          <button
            type="submit"
            className={`${remoteButton} !border-accent !bg-accent text-white`}
            disabled={sending || dictating || loadingAttachments || !connectionId || (!draft.trim() && !attachments.length)}
          >
            {sending ? (
              <Loader2 size={16} className="animate-spin" />
            ) : (
              <Send size={16} />
            )}
            {t("remote.send")}
          </button>
        </div>
      </form>
    </main>
  );
}

function RemoteChatMessage({ message, onMore }: { message: ChatMessage; onMore: () => void }) {
  const { t } = useTranslation();
  return <article className={`remote-message ${message.role === 'user' ? 'remote-message-user' : 'remote-answer'}`}>
    <p className="mb-3 text-xs font-semibold text-text-tertiary">{message.role === 'user' ? t('remote.you') : 'Nexa'}</p>
    {message.role === 'user' ? <div className="whitespace-pre-wrap break-words text-sm leading-7">{message.content}</div>
      : <StreamingMarkdown content={message.content} isStreaming={false} reduceMotion />}
    {[...message.content].length < message.totalChars && <button className={`${remoteButton} mt-3`} onClick={onMore}>
      <ArrowDown size={14} />{t('remote.moreText')}
    </button>}
  </article>;
}
