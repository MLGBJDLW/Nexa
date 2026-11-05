import { fromMarkdown } from 'mdast-util-from-markdown';
import type { ConversationMessage } from '../types/conversation';

export interface FollowupSuggestion { label: string; prompt: string }

/** Local choices keep completed conversations usable when a model omits metadata. */
export function defaultFollowupSuggestions(t: (key: 'chat.followupExplainLabel' | 'chat.followupExplainPrompt' | 'chat.followupNextLabel' | 'chat.followupNextPrompt') => string): FollowupSuggestion[] {
  return [{ label: t('chat.followupExplainLabel'), prompt: t('chat.followupExplainPrompt') },
    { label: t('chat.followupNextLabel'), prompt: t('chat.followupNextPrompt') }];
}

/** Suggestions are inert message data. Only an explicit click or send executes one. */
export function parseFollowupSuggestions(source: string): FollowupSuggestion[] {
  if (source.length > 4096) return [];
  try {
    // Older prompts asked for multiple JSON objects. Accept that exact JSONL
    // shape as well as the canonical array, without repairing arbitrary JSON.
    const trimmed = source.trim();
    const items: unknown = trimmed.startsWith('[')
      ? JSON.parse(trimmed)
      : trimmed.split(/\r?\n/).filter(line => line.trim()).map(line => JSON.parse(line));
    if (!Array.isArray(items) || items.length < 1 || items.length > 3) return [];
    const result: FollowupSuggestion[] = [];
    for (const item of items) {
      if (!item || typeof item !== 'object' || typeof item.label !== 'string' || typeof item.prompt !== 'string') return [];
      const label = item.label.trim();
      const prompt = item.prompt.trim();
      if (!label || label.length > 80 || !prompt || prompt.length > 600 || prompt.startsWith('/') || /[\u0000-\u001f]/.test(label + prompt)) return [];
      if (result.some(previous => previous.prompt === prompt)) return [];
      result.push({ label, prompt });
    }
    return result;
  } catch { return []; }
}

/** Speech is a projection: metadata never mutates the stored message or draft. */
export function stripFollowupBlocks(content: string): string {
  const blocks = fromMarkdown(content).children.filter(node => node.type === 'code' && node.lang?.toLowerCase() === 'nexa-followups');
  for (const block of blocks.reverse()) {
    content = content.slice(0, block.position!.start.offset) + content.slice(block.position!.end.offset);
  }
  return content;
}

type FollowupMessage = Pick<ConversationMessage, 'id' | 'content'> & { role: string; toolCalls?: ConversationMessage['toolCalls']; artifacts?: ConversationMessage['artifacts'] };
export function latestFollowupSuggestions(messages: FollowupMessage[], fallback: FollowupSuggestion[] = []): { key: string; items: FollowupSuggestion[] } | null {
  const message = [...messages].reverse().find(item => item.role === 'user' || item.role === 'assistant');
  if (!message || message.role !== 'assistant' || message.toolCalls?.length) return null;
  if (message.artifacts && !Array.isArray(message.artifacts) && message.artifacts.phase === 'commentary') return null;
  if (!message.content.trim()) return null;
  // Parse Markdown structure so examples inside longer fences cannot become actions.
  const children = fromMarkdown(message.content).children;
  const last = children[children.length - 1];
  if (last?.type !== 'code' || last.lang?.toLowerCase() !== 'nexa-followups') return fallback.length ? { key: message.id, items: fallback } : null;
  const raw = message.content.slice(last.position?.start.offset, last.position?.end.offset);
  const lines = raw.trimEnd().split(/\r?\n/);
  const opening = /^ {0,3}(`{3,}|~{3,})nexa-followups\s*$/i.exec(lines[0]);
  const closing = lines[lines.length - 1]?.trim();
  if (!opening || !closing || closing.length < opening[1].length || [...closing].some(char => char !== opening[1][0])) return fallback.length ? { key: message.id, items: fallback } : null;
  const items = parseFollowupSuggestions(last.value);
  return items.length ? { key: message.id, items } : fallback.length ? { key: message.id, items: fallback } : null;
}
