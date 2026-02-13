import { useRef, useEffect, useMemo } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { MessageCircle } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { ToolCallCard } from './ToolCallCard';
import type { ConversationMessage } from '../../types/conversation';

/* ------------------------------------------------------------------ */
/*  Types                                                              */
/* ------------------------------------------------------------------ */

interface ToolCallEvent {
  callId: string;
  toolName: string;
  arguments: string;
  status: 'running' | 'done' | 'error';
  content?: string;
  isError?: boolean;
  artifacts?: Record<string, unknown>;
}

interface ChatMessagesProps {
  messages: ConversationMessage[];
  streamText: string;
  toolCalls: ToolCallEvent[];
  isStreaming: boolean;
}

/* ------------------------------------------------------------------ */
/*  Basic markdown renderer (no external lib)                          */
/* ------------------------------------------------------------------ */

function renderSimpleMarkdown(text: string): React.ReactNode[] {
  // Split by code blocks first
  const parts = text.split(/(```[\s\S]*?```)/g);

  return parts.map((part, i) => {
    // Code block
    if (part.startsWith('```') && part.endsWith('```')) {
      const inner = part.slice(3, -3);
      const newlineIdx = inner.indexOf('\n');
      const code = newlineIdx >= 0 ? inner.slice(newlineIdx + 1) : inner;
      return (
        <pre
          key={i}
          className="bg-surface-0 border border-border rounded-md px-3 py-2 my-2 text-xs overflow-x-auto"
        >
          <code>{code}</code>
        </pre>
      );
    }

    // Inline formatting
    const elements: React.ReactNode[] = [];
    const inlineParts = part.split(/(\*\*[^*]+\*\*|`[^`]+`)/g);

    for (let j = 0; j < inlineParts.length; j++) {
      const seg = inlineParts[j];
      if (seg.startsWith('**') && seg.endsWith('**')) {
        elements.push(<strong key={`${i}-${j}`}>{seg.slice(2, -2)}</strong>);
      } else if (seg.startsWith('`') && seg.endsWith('`')) {
        elements.push(
          <code
            key={`${i}-${j}`}
            className="bg-surface-0 border border-border rounded px-1 py-0.5 text-xs"
          >
            {seg.slice(1, -1)}
          </code>,
        );
      } else {
        // Split by newlines for paragraphs
        const lines = seg.split('\n');
        for (let k = 0; k < lines.length; k++) {
          if (k > 0) elements.push(<br key={`${i}-${j}-br-${k}`} />);
          elements.push(<span key={`${i}-${j}-${k}`}>{lines[k]}</span>);
        }
      }
    }

    return <span key={i}>{elements}</span>;
  });
}

/* ------------------------------------------------------------------ */
/*  Message bubble                                                     */
/* ------------------------------------------------------------------ */

function MessageBubble({ msg }: { msg: ConversationMessage }) {
  const isUser = msg.role === 'user';

  if (msg.role === 'tool' || msg.role === 'system') return null;

  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
      className={`flex ${isUser ? 'justify-end' : 'justify-start'} mb-3`}
    >
      <div
        className={`max-w-[80%] rounded-lg px-3.5 py-2.5 text-sm leading-relaxed
          ${isUser
            ? 'bg-accent/20 text-text-primary'
            : 'bg-surface-2 text-text-primary'
          }`}
      >
        {isUser ? (
          <span className="whitespace-pre-wrap">{msg.content}</span>
        ) : (
          <div className="prose-chat">{renderSimpleMarkdown(msg.content)}</div>
        )}
      </div>
    </motion.div>
  );
}

/* ------------------------------------------------------------------ */
/*  Component                                                          */
/* ------------------------------------------------------------------ */

export function ChatMessages({ messages, streamText, toolCalls, isStreaming }: ChatMessagesProps) {
  const { t } = useTranslation();
  const bottomRef = useRef<HTMLDivElement>(null);

  // Build tool call map from messages for completed tool calls
  const messageToolCalls = useMemo(() => {
    const map = new Map<number, ConversationMessage[]>();
    for (let i = 0; i < messages.length; i++) {
      const msg = messages[i];
      if (msg.role === 'assistant' && msg.toolCalls.length > 0) {
        const toolResults: ConversationMessage[] = [];
        for (let j = i + 1; j < messages.length; j++) {
          if (messages[j].role === 'tool') {
            toolResults.push(messages[j]);
          } else {
            break;
          }
        }
        map.set(i, toolResults);
      }
    }
    return map;
  }, [messages]);

  // Auto-scroll on new messages or streaming
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages, streamText, toolCalls]);

  // Empty state
  if (messages.length === 0 && !isStreaming) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <div className="text-center">
          <div className="p-4 rounded-2xl bg-surface-2 text-text-tertiary inline-block mb-4">
            <MessageCircle className="h-8 w-8" />
          </div>
          <p className="text-sm text-text-tertiary">{t('chat.placeholder')}</p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto px-4 py-4">
      <AnimatePresence initial={false}>
        {messages.map((msg, idx) => {
          // Skip tool result messages (rendered inline with tool calls)
          if (msg.role === 'tool' || msg.role === 'system') return null;

          return (
            <div key={msg.id}>
              <MessageBubble msg={msg} />

              {/* Show tool call cards after assistant messages with tool calls */}
              {msg.role === 'assistant' && msg.toolCalls.length > 0 && (
                <div className="mb-3">
                  {msg.toolCalls.map((tc) => {
                    const toolResult = messageToolCalls.get(idx)?.find(
                      (tr) => tr.toolCallId === tc.id,
                    );
                    return (
                      <ToolCallCard
                        key={tc.id}
                        toolName={tc.name}
                        arguments={tc.arguments}
                        status={toolResult ? 'done' : 'running'}
                        content={toolResult?.content}
                      />
                    );
                  })}
                </div>
              )}
            </div>
          );
        })}
      </AnimatePresence>

      {/* Streaming tool calls */}
      {isStreaming && toolCalls.length > 0 && (
        <div className="mb-3">
          {toolCalls.map((tc) => (
            <ToolCallCard
              key={tc.callId}
              toolName={tc.toolName}
              arguments={tc.arguments}
              status={tc.status}
              content={tc.content}
              isError={tc.isError}
              artifacts={tc.artifacts}
            />
          ))}
        </div>
      )}

      {/* Streaming text */}
      {isStreaming && streamText && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          className="flex justify-start mb-3"
        >
          <div className="max-w-[80%] rounded-lg px-3.5 py-2.5 text-sm leading-relaxed bg-surface-2 text-text-primary">
            <div className="prose-chat">
              {renderSimpleMarkdown(streamText)}
              <span className="inline-block w-1.5 h-4 bg-accent animate-pulse ml-0.5 align-text-bottom rounded-sm" />
            </div>
          </div>
        </motion.div>
      )}

      {/* Thinking indicator */}
      {isStreaming && !streamText && toolCalls.length === 0 && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          className="flex justify-start mb-3"
        >
          <div className="rounded-lg px-3.5 py-2.5 bg-surface-2">
            <div className="flex items-center gap-2 text-sm text-text-tertiary">
              <div className="flex gap-1">
                <span className="w-1.5 h-1.5 rounded-full bg-text-tertiary animate-bounce" style={{ animationDelay: '0ms' }} />
                <span className="w-1.5 h-1.5 rounded-full bg-text-tertiary animate-bounce" style={{ animationDelay: '150ms' }} />
                <span className="w-1.5 h-1.5 rounded-full bg-text-tertiary animate-bounce" style={{ animationDelay: '300ms' }} />
              </div>
              {t('chat.thinking')}
            </div>
          </div>
        </motion.div>
      )}

      <div ref={bottomRef} />
    </div>
  );
}
