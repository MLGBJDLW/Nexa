import { useState, useCallback, useRef } from 'react';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import * as api from './api';
import type { AgentEvent } from '../types';

interface ToolCallEvent {
  callId: string;
  toolName: string;
  arguments: string;
  status: 'running' | 'done' | 'error';
  content?: string;
  isError?: boolean;
  artifacts?: Record<string, unknown>;
}

interface UseAgentStreamReturn {
  send: (conversationId: string, message: string) => Promise<void>;
  stop: (conversationId: string) => Promise<void>;
  isStreaming: boolean;
  streamText: string;
  toolCalls: ToolCallEvent[];
  error: string | null;
  reset: () => void;
}

export function useAgentStream(): UseAgentStreamReturn {
  const [isStreaming, setIsStreaming] = useState(false);
  const [streamText, setStreamText] = useState('');
  const [toolCalls, setToolCalls] = useState<ToolCallEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const unlistenRef = useRef<UnlistenFn | null>(null);

  const reset = useCallback(() => {
    setStreamText('');
    setToolCalls([]);
    setError(null);
  }, []);

  const send = useCallback(async (conversationId: string, message: string) => {
    // Cleanup previous listener
    if (unlistenRef.current) {
      unlistenRef.current();
    }

    setIsStreaming(true);
    setError(null);
    setStreamText('');
    setToolCalls([]);

    // Listen for agent events BEFORE sending the command
    unlistenRef.current = await listen<AgentEvent>('agent:event', (event) => {
      const data = event.payload;
      switch (data.type) {
        case 'textDelta':
          setStreamText(prev => prev + (data.delta || ''));
          break;
        case 'toolCallStart':
          setToolCalls(prev => [...prev, {
            callId: data.callId!,
            toolName: data.toolName!,
            arguments: data.arguments || '',
            status: 'running',
          }]);
          break;
        case 'toolCallResult':
          setToolCalls(prev => prev.map(tc =>
            tc.callId === data.callId
              ? { ...tc, status: data.isError ? 'error' : 'done', content: data.content, isError: data.isError, artifacts: data.artifacts }
              : tc
          ));
          // After tool result, reset text for new LLM response
          setStreamText('');
          break;
        case 'done':
          setIsStreaming(false);
          if (unlistenRef.current) {
            unlistenRef.current();
            unlistenRef.current = null;
          }
          break;
        case 'error':
          setError(data.content || data.delta || 'Unknown error');
          setIsStreaming(false);
          if (unlistenRef.current) {
            unlistenRef.current();
            unlistenRef.current = null;
          }
          break;
      }
    });

    // Send the message
    try {
      await api.agentChat(conversationId, message);
    } catch (err) {
      setError(String(err));
      setIsStreaming(false);
    }
  }, []);

  const stop = useCallback(async (conversationId: string) => {
    try {
      await api.agentStop(conversationId);
    } catch (err) {
      // Ignore errors on stop
    }
    setIsStreaming(false);
    if (unlistenRef.current) {
      unlistenRef.current();
      unlistenRef.current = null;
    }
  }, []);

  return { send, stop, isStreaming, streamText, toolCalls, error, reset };
}
