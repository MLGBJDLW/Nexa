import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowDown, CircleStop, Loader2, Plus, Send } from "lucide-react";
import { useTranslation } from "../../i18n";
import { RemoteClient } from "./remoteClient";
import { remoteButton, remoteField } from "./remoteUi";

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
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [conversationId, setConversationId] = useState(
    () =>
      localStorage.getItem(
        `nexa.remote.chat.${client.paired.manifest.serverId}`,
      ) || "",
  );
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [beforeOrder, setBeforeOrder] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [stream, setStream] = useState("");
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
    const encoder = new TextEncoder();
    let paint: ReturnType<typeof setTimeout> | null = null;
    const retired = new Set<string>();
    setStream("");
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
    const valid = () => !stopped && scope.current === generation;
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
    setSending(true);
    setError("");
    try {
      const conversation = await client.rpc<Conversation>("chat.create", {
        connectionId,
      });
      setConversationId(conversation.id);
      setConversations((current) => [conversation, ...current]);
      pendingSend.current = null;
    } catch (error) {
      errorText(error);
    } finally {
      setSending(false);
    }
  }
  async function send() {
    if (sending || !draft.trim() || !connectionId) return;
    setSending(true);
    setError("");
    try {
      let id = conversationId;
      if (!id) {
        const conversation = await client.rpc<Conversation>("chat.create", {
          connectionId,
        });
        id = conversation.id;
        setConversationId(id);
        setConversations((current) => [conversation, ...current]);
      }
      const previous = pendingSend.current;
      const request =
        previous &&
        previous.message === draft &&
        previous.conversationId === id &&
        previous.connectionId === connectionId
          ? previous
          : {
              message: draft,
              id: crypto.randomUUID(),
              conversationId: id,
              connectionId,
            };
      pendingSend.current = request;
      await client.rpc("chat.start", {
        conversationId: id,
        connectionId,
        message: request.message,
        idempotencyKey: request.id,
      });
      setDraft("");
      pendingSend.current = null;
      setRunning(true);
      refreshRef.current();
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
    try {
      const page = await client.rpc<MessagePage>("chat.read", {
        conversationId,
        beforeOrder,
      });
      setMessages((current) => [...page.messages, ...current]);
      setBeforeOrder(page.beforeOrder);
    } catch (error) {
      errorText(error);
    }
  }
  async function more(message: ChatMessage) {
    try {
      const page = await client.rpc<{ content: string; totalChars: number }>(
        "chat.message",
        {
          conversationId,
          messageId: message.id,
          offset: [...message.content].length,
        },
      );
      setMessages((current) =>
        current.map((item) =>
          item.id === message.id
            ? { ...item, content: item.content + page.content }
            : item,
        ),
      );
    } catch (error) {
      errorText(error);
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
            setConversationId(event.target.value);
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
        aria-label={t("remote.model")}
        className={remoteField}
        value={connectionId}
        onChange={(event) => setConnectionId(event.target.value)}
      >
        {connections.map((item) => (
          <option key={item.id} value={item.id}>
            {item.name} · {item.model}
          </option>
        ))}
      </select>
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
            <div className="whitespace-pre-wrap break-words text-sm leading-7">
              {message.content}
            </div>
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
            <div className="whitespace-pre-wrap break-words text-sm leading-7">
              {stream}
            </div>
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
                onChange={(event) =>
                  setAnswers((current) => ({
                    ...current,
                    [item.interactionId]: {
                      ...current[item.interactionId],
                      [question.id]: event.target.value
                        ? [event.target.value]
                        : [],
                    },
                  }))
                }
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
          onChange={(event) => setDraft(event.target.value)}
        />
        <div className="flex justify-end gap-2">
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
            disabled={sending || !connectionId || !draft.trim()}
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
