import { useState, useRef, useCallback, useEffect } from 'react';
import { motion } from 'framer-motion';
import { Send, Square } from 'lucide-react';
import { useTranslation } from '../../i18n';

/* ------------------------------------------------------------------ */
/*  Types                                                              */
/* ------------------------------------------------------------------ */

interface ChatInputProps {
  onSend: (message: string) => void;
  onStop: () => void;
  isStreaming: boolean;
  disabled: boolean;
}

/* ------------------------------------------------------------------ */
/*  Component                                                          */
/* ------------------------------------------------------------------ */

export function ChatInput({ onSend, onStop, isStreaming, disabled }: ChatInputProps) {
  const { t } = useTranslation();
  const [value, setValue] = useState('');
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Auto-resize textarea
  const adjustHeight = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = 'auto';
    const lineHeight = 22;
    const maxHeight = lineHeight * 6 + 16; // ~6 lines + padding
    el.style.height = `${Math.min(el.scrollHeight, maxHeight)}px`;
  }, []);

  useEffect(() => {
    adjustHeight();
  }, [value, adjustHeight]);

  const handleSend = useCallback(() => {
    const trimmed = value.trim();
    if (!trimmed) return;
    onSend(trimmed);
    setValue('');
    // Reset height after send
    setTimeout(() => {
      if (textareaRef.current) {
        textareaRef.current.style.height = 'auto';
      }
    }, 0);
  }, [value, onSend]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        if (!isStreaming && !disabled) {
          handleSend();
        }
      }
    },
    [handleSend, isStreaming, disabled],
  );

  return (
    <div className="border-t border-border bg-surface-1 px-4 py-3">
      <div className="flex items-end gap-2">
        <textarea
          ref={textareaRef}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={t('chat.placeholder')}
          disabled={disabled}
          rows={1}
          className="flex-1 resize-none bg-surface-0 border border-border rounded-lg
            text-sm text-text-primary placeholder:text-text-tertiary
            px-3.5 py-2.5 outline-none
            transition-all duration-fast ease-out
            hover:border-border-hover
            focus:border-accent focus:ring-1 focus:ring-accent/30
            disabled:opacity-40 disabled:pointer-events-none"
        />

        {isStreaming ? (
          <motion.button
            whileTap={{ scale: 0.95 }}
            onClick={onStop}
            className="shrink-0 h-10 w-10 flex items-center justify-center
              rounded-lg bg-danger/10 text-danger hover:bg-danger/20
              transition-colors duration-fast ease-out cursor-pointer"
            aria-label={t('chat.stop')}
          >
            <Square className="h-4 w-4" />
          </motion.button>
        ) : (
          <motion.button
            whileTap={{ scale: 0.95 }}
            onClick={handleSend}
            disabled={disabled || !value.trim()}
            className="shrink-0 h-10 w-10 flex items-center justify-center
              rounded-lg bg-accent text-white hover:bg-accent-hover
              transition-colors duration-fast ease-out cursor-pointer
              disabled:opacity-40 disabled:pointer-events-none"
            aria-label={t('chat.send')}
          >
            <Send className="h-4 w-4" />
          </motion.button>
        )}
      </div>
    </div>
  );
}
