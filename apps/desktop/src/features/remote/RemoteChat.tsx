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
import type { ImageAttachment } from '../../types/conversation';

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
interface RunEvent {
  runId: string;
  eventSeq: number;
  kind: string;
  label: string;
  visibility: string;
  payload: Record<string, any>;
}
interface RunPage {
  run: { id: string; status: string } | null;
  events: RunEvent[];
  hasMore: boolean;
  nextSequence: number | null;
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
  const [stream, setStream] = useState("");
  const [thinking, setThinking] = useState('');
  const [tools, setTools] = useState<Record<string, Record<string, any>>>({});
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
    let runId: string | null = null;
    let sequence = 0;
    const waiting = new Map<number, RunEvent>();
    const blocks = new Map<string, { text: string; bytes: number }>();
    const reasoning = new Map<string, { text: string; bytes: number }>();
    const encoder = new TextEncoder();
    let paint: ReturnType<typeof setTimeout> | null = null;
    const retired = new Set<string>();
    setStream("");
    setThinking(''); setTools({});
    setProgress("");
    setRunning(false);
    setMessages([]);
    setApprovals([]);
    setInteractions([]);
    setBeforeOrder(null);
    if (!conversationId) return;
    localStorage.setItem(
      `nexa.remote.chat.${client.paired.manifest.serverId}`,
      conversationId,
    );
    const valid = () =>
      !stopped &&
      scope.current === generation &&
      selectedConversation.current === conversationId;
    const publishStream = () => {
      if (paint) return;
      paint = setTimeout(() => {
        paint = null;
        if (valid())
          setStream(
            [...blocks.values()]
              .map((block) => block.text)
              .join("\n\n")
              .slice(-128_000),
          );
      }, 60);
    };
    const read = async () => {
      const result = await client.rpc<MessagePage>("chat.read", {
        conversationId,
      });
      if (valid()) {
        setMessages(result.messages);
        setBeforeOrder(result.beforeOrder);
      }
    };
    const questions = async () => {
      const [nextApprovals, nextInteractions] = await Promise.all([
        client.rpc<Approval[]>("approvals.list"),
        client.rpc<Interaction[]>("interactions.list", { conversationId }),
      ]);
      if (valid()) {
        setApprovals(nextApprovals.filter((item) => item.runId === runId));
        setInteractions(nextInteractions);
      }
    };
    const apply = (event: RunEvent) => {
      sequence = event.eventSeq;
      if (event.visibility === "user" && event.label) setProgress(event.label);
      const payload = event.payload;
      if (event.visibility === 'user' && event.kind.startsWith('tool') && payload.run?.callId) {
        setTools(current => { const next: Record<string, Record<string, any>> = { ...current, [payload.run.callId]:payload.run }; return Object.fromEntries(Object.entries(next).slice(-64)); });
      }
      if (payload.channel === 'thinking' && (event.kind === 'outputDelta' || event.kind === 'outputSnapshot')) {
        const previous = reasoning.get(payload.blockId) || { text:'', bytes:0 };
        if (event.kind === 'outputSnapshot' || previous.bytes === payload.offset) {
          const text = (event.kind === 'outputSnapshot' ? String(payload.text ?? '') : previous.text + String(payload.delta ?? '')).slice(0, 128_000);
          reasoning.set(payload.blockId, { text, bytes:encoder.encode(text).byteLength });
          setThinking([...reasoning.values()].map(block => block.text).join('\n\n').slice(-128_000));
        }
      }
      if (event.kind === "outputDelta" && payload.channel === "answer") {
        const previous = blocks.get(payload.blockId) || { text: "", bytes: 0 };
        if (
          previous.bytes === payload.offset &&
          previous.text.length < 128_000
        ) {
          const delta = String(payload.delta || "");
          blocks.set(payload.blockId, {
            text: previous.text + delta,
            bytes: previous.bytes + encoder.encode(delta).byteLength,
          });
          publishStream();
        }
      }
      if (event.kind === "outputSnapshot" && payload.channel === "answer") {
        const text = String(payload.text ?? payload.content ?? "");
        blocks.set(payload.blockId, {
          text,
          bytes: encoder.encode(text).byteLength,
        });
        publishStream();
      }
      if (event.kind === "streamReset" && payload.discardSample) {
        blocks.clear();
        reasoning.clear(); setThinking('');
        setStream("");
      }
      if (event.kind === "done" || event.kind === "error") {
        setRunning(false);
        const completedRun = event.runId;
        void read()
          .then(() => {
            if (valid() && runId === completedRun) {
              setStream("");
              blocks.clear();
            }
          })
          .catch(errorText);
        void list().catch(() => {});
      }
      if (
        event.kind === "approvalRequested" ||
        event.kind === "approvalResolved" ||
        event.kind === "interactionRequested"
      )
        void questions().catch(errorText);
    };
    const receive = (event: RunEvent) => {
      if (!valid()) return;
      if (retired.has(event.runId)) return;
      if (runId !== event.runId) {
        if (runId) retired.add(runId);
        runId = event.runId;
        sequence = 0;
        waiting.clear();
        blocks.clear();
        reasoning.clear(); setThinking(''); setTools({});
        setStream("");
        setRunning(true);
      }
      if (event.eventSeq <= sequence) return;
      waiting.set(event.eventSeq, event);
      while (waiting.has(sequence + 1)) {
        const next = waiting.get(sequence + 1)!;
        waiting.delete(sequence + 1);
        apply(next);
      }
      if (waiting.size > 256) waiting.clear();
      if (event.eventSeq > sequence + 1 && !syncing) void sync();
    };
    const sync = async () => {
      if (!valid()) return;
      if (syncing) {
        syncAgain = true;
        return;
      }
      syncing = true;
      try {
        let more = true;
        while (more && valid()) {
          const page = await client.rpc<RunPage>("chat.resume", {
            conversationId,
            runId,
            afterSequence: sequence,
          });
          if (!valid()) return;
          if (page.run) {
            // A newer desktop run can arrive while an older replay request is in flight.
            if (retired.has(page.run.id)) {
              syncAgain = true;
              return;
            }
            if (runId !== page.run.id) {
              if (runId) retired.add(runId);
              runId = page.run.id;
              sequence = 0;
              waiting.clear();
              blocks.clear();
              reasoning.clear(); setThinking(''); setTools({});
              setStream("");
            }
            setRunning(
              [
                "queued",
                "running",
                "waiting_approval",
                "waiting_input",
              ].includes(page.run.status),
            );
          }
          for (const event of page.events) receive(event);
          more = page.hasMore;
          if (
            more &&
            (!page.nextSequence ||
              (page.nextSequence <= sequence && !page.events.length))
          )
            break;
        }
        await Promise.all([read(), questions()]);
      } catch (error) {
        if (valid() && client.state.phase === "connected") errorText(error);
      } finally {
        syncing = false;
        if (syncAgain) {
          syncAgain = false;
          void sync();
        }
      }
    };
    refreshRef.current = () => {
      void sync();
    };
    void sync();
    const off = client.subscribe((event) => {
      if (
        event.event === "agent://run-event" &&
        event.payload.conversationId === conversationId
      )
        receive(event.payload.runEvent);
      else if (event.event === "connection:resync") {
        void list().catch(() => {});
        void sync();
      }
    });
    const timer = setInterval(() => {
      if (client.state.phase === "connected") void questions().catch(() => {});
    }, 3000);
    return () => {
      stopped = true;
      off();
      clearInterval(timer);
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
      <div className="space-y-5" aria-live="polite">
        {messages.map((message) => (
          <article
            key={message.id}
            className={`rounded-2xl p-4 ${message.role === "user" ? "ml-8 border border-accent/15 bg-accent/5" : "border border-border bg-surface-1"}`}
          >
            <p className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-text-tertiary">
              {message.role === "user" ? t("remote.you") : "Nexa"}
            </p>
            {message.role === 'user' ? <div className="whitespace-pre-wrap break-words text-sm leading-7">{message.content}</div>
              : <StreamingMarkdown content={message.content} isStreaming={false} reduceMotion />}
            {[...message.content].length < message.totalChars && (
              <button
                className={`${remoteButton} mt-3`}
                onClick={() => void more(message)}
              >
                <ArrowDown size={14} />
                {t("remote.moreText")}
              </button>
            )}
          </article>
        ))}
        {stream && (
          <article className="rounded-2xl border border-accent/20 bg-surface-1 p-4">
            <p className="mb-2 text-xs font-medium text-accent">Nexa</p>
            <StreamingMarkdown content={stream} isStreaming={running} reduceMotion />
          </article>
        )}
      </div>
      {progress && (
        <p role="status" className="text-xs leading-5 text-text-secondary">
          {running && (
            <Loader2 size={13} className="mr-2 inline animate-spin" />
          )}
          {progress}
        </p>
      )}
      {(thinking || Object.keys(tools).length > 0) && <details className="rounded-xl border border-border bg-surface-1 p-3 text-sm">
        <summary className="cursor-pointer text-text-secondary">{t('remote.activity')}</summary>
        {thinking && <div className="mt-3 max-h-80 overflow-auto"><StreamingMarkdown content={thinking} isStreaming={running} reduceMotion /></div>}
        {Object.entries(tools).map(([id, tool]) => <details key={id} className="mt-2 rounded-lg border border-border p-2">
          <summary className="cursor-pointer">{tool.toolName} · {tool.status}</summary>
          {tool.progressNote && <p className="mt-2 text-xs">{tool.progressNote}</p>}
          {tool.arguments && <pre className="my-2 max-h-40 overflow-auto whitespace-pre-wrap break-all text-xs">{tool.arguments}</pre>}
          {tool.content && <div className="max-h-80 overflow-auto"><StreamingMarkdown content={tool.content} isStreaming={false} reduceMotion /></div>}
        </details>)}
      </details>}
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
      {!messages.length && !stream && !loading && (
        <div className="py-8 text-center text-sm leading-7 text-text-secondary">
          {t("remote.chatEmpty")}
        </div>
      )}
      <form
        className="sticky bottom-3 mt-auto rounded-2xl border border-border bg-surface-0 p-3 shadow-lg"
        onSubmit={(event) => {
          event.preventDefault();
          void send();
        }}
      >
        <textarea
          aria-label={t("remote.message")}
          className="max-h-52 min-h-24 w-full resize-y bg-transparent p-1 text-sm leading-6 outline-none"
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
