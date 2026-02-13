import { useState, useEffect, useCallback, useRef } from 'react';
import { useNavigate } from 'react-router-dom';
import { X, Plus, Settings } from 'lucide-react';
import { toast } from 'sonner';
import * as api from '../../lib/api';
import { useAgentStream } from '../../lib/useAgentStream';
import { useTranslation } from '../../i18n';
import { ChatMessages } from './ChatMessages';
import { ChatInput } from './ChatInput';
import { EmptyState } from '../ui/EmptyState';
import type { ConversationMessage, AgentConfig } from '../../types/conversation';

/* ------------------------------------------------------------------ */
/*  Types                                                              */
/* ------------------------------------------------------------------ */

interface ChatPanelProps {
  /** Initial message to send when panel opens (e.g., search query) */
  initialMessage?: string;
  /** Called when user wants to close the panel */
  onClose: () => void;
  /** Additional class names */
  className?: string;
}

/* ------------------------------------------------------------------ */
/*  Component                                                          */
/* ------------------------------------------------------------------ */

export function ChatPanel({ initialMessage, onClose, className }: ChatPanelProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();

  /* ── State ──────────────────────────────────────────────────────── */
  const [messages, setMessages] = useState<ConversationMessage[]>([]);
  const [defaultConfig, setDefaultConfig] = useState<AgentConfig | null>(null);
  const [configLoading, setConfigLoading] = useState(true);
  const [conversationId, setConversationId] = useState<string | null>(null);

  const { send, stop, isStreaming, streamText, toolCalls, error, reset } = useAgentStream();

  // Track the last initialMessage we auto-sent, to avoid re-sending
  const sentInitialRef = useRef<string | null>(null);

  /* ── Load default agent config ──────────────────────────────────── */
  const loadDefaultConfig = useCallback(async () => {
    try {
      const configs = await api.listAgentConfigs();
      const def = configs.find((c) => c.isDefault) ?? configs[0] ?? null;
      setDefaultConfig(def);
    } catch {
      setDefaultConfig(null);
    } finally {
      setConfigLoading(false);
    }
  }, []);

  useEffect(() => {
    loadDefaultConfig();
  }, [loadDefaultConfig]);

  /* ── Reload messages when streaming completes ───────────────────── */
  useEffect(() => {
    if (!isStreaming && conversationId && messages.length > 0) {
      api.getConversation(conversationId).then(([, msgs]) => {
        setMessages(msgs);
      }).catch(() => {});
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isStreaming]);

  /* ── Show error toast ───────────────────────────────────────────── */
  useEffect(() => {
    if (error) toast.error(error);
  }, [error]);

  /* ── Auto-send initialMessage ───────────────────────────────────── */
  useEffect(() => {
    if (
      initialMessage &&
      initialMessage.trim() &&
      defaultConfig &&
      !configLoading &&
      sentInitialRef.current !== initialMessage
    ) {
      sentInitialRef.current = initialMessage;
      handleSendMessage(initialMessage);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialMessage, defaultConfig, configLoading]);

  /* ── Handlers ───────────────────────────────────────────────────── */

  const handleSendMessage = useCallback(
    async (message: string) => {
      if (!defaultConfig) return;

      let convId = conversationId;

      // Auto-create conversation if none active
      if (!convId) {
        try {
          const conv = await api.createConversation(
            defaultConfig.provider,
            defaultConfig.model,
          );
          convId = conv.id;
          setConversationId(convId);
        } catch (e) {
          toast.error(`Create conversation failed: ${String(e)}`);
          return;
        }
      }

      // Add optimistic user message
      const optimisticMsg: ConversationMessage = {
        id: `temp-${Date.now()}`,
        conversationId: convId,
        role: 'user',
        content: message,
        toolCallId: null,
        toolCalls: [],
        tokenCount: 0,
        createdAt: new Date().toISOString(),
        sortOrder: messages.length,
      };
      setMessages((prev) => [...prev, optimisticMsg]);

      await send(convId, message);
    },
    [conversationId, defaultConfig, messages.length, send],
  );

  const handleStop = useCallback(() => {
    if (conversationId) {
      stop(conversationId);
    }
  }, [conversationId, stop]);

  const handleNewChat = useCallback(() => {
    setConversationId(null);
    setMessages([]);
    sentInitialRef.current = null;
    reset();
  }, [reset]);

  /* ── No provider configured ─────────────────────────────────────── */
  if (!configLoading && !defaultConfig) {
    return (
      <div className={`flex flex-col h-full ${className ?? ''}`}>
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 bg-surface-2 border-b border-border">
          <span className="text-sm font-semibold text-text-primary">
            {t('chat.aiAssistant')}
          </span>
          <button
            onClick={onClose}
            className="rounded-md p-1.5 text-text-tertiary hover:bg-surface-3 hover:text-text-secondary transition-colors cursor-pointer"
            aria-label={t('chat.closePanel')}
          >
            <X size={16} />
          </button>
        </div>

        <div className="flex-1 flex items-center justify-center p-4">
          <EmptyState
            icon={<Settings className="h-8 w-8" />}
            title={t('chat.noProvider')}
            description={t('chat.noProviderDesc')}
            action={{
              label: t('chat.configureProvider'),
              onClick: () => navigate('/settings'),
            }}
          />
        </div>
      </div>
    );
  }

  /* ── Render ──────────────────────────────────────────────────────── */
  return (
    <div className={`flex flex-col h-full ${className ?? ''}`}>
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 bg-surface-2 border-b border-border">
        <span className="text-sm font-semibold text-text-primary">
          {t('chat.aiAssistant')}
        </span>
        <div className="flex items-center gap-1">
          <button
            onClick={handleNewChat}
            className="rounded-md px-2 py-1.5 text-xs font-medium text-text-tertiary hover:bg-surface-3 hover:text-text-secondary transition-colors cursor-pointer flex items-center gap-1"
            aria-label={t('chat.newChatShort')}
          >
            <Plus size={13} />
            {t('chat.newChatShort')}
          </button>
          <button
            onClick={onClose}
            className="rounded-md p-1.5 text-text-tertiary hover:bg-surface-3 hover:text-text-secondary transition-colors cursor-pointer"
            aria-label={t('chat.closePanel')}
          >
            <X size={16} />
          </button>
        </div>
      </div>

      {/* Messages area */}
      <ChatMessages
        messages={messages}
        streamText={streamText}
        toolCalls={toolCalls}
        isStreaming={isStreaming}
      />

      {/* Input */}
      <ChatInput
        onSend={handleSendMessage}
        onStop={handleStop}
        isStreaming={isStreaming}
        disabled={!defaultConfig || configLoading}
      />
    </div>
  );
}
