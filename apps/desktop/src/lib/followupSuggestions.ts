import { fromMarkdown } from 'mdast-util-from-markdown';
import type { ConversationMessage } from '../types/conversation';

export interface FollowupSuggestion { label: string; prompt: string }

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
    if (!Array.isArray(items) || items.length < 2 || items.length > 3) return [];
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

export function latestFollowupSuggestions(messages: ConversationMessage[]): { key: string; items: FollowupSuggestion[] } | null {
  const message = [...messages].reverse().find(item => item.role === 'user' || item.role === 'assistant');
  if (!message || message.role !== 'assistant' || message.toolCalls.length) return null;
  if (message.artifacts && !Array.isArray(message.artifacts) && message.artifacts.phase === 'commentary') return null;
  // Parse Markdown structure so examples inside longer fences cannot become actions.
  const children = fromMarkdown(message.content).children;
  const last = children[children.length - 1];
  if (last?.type !== 'code' || last.lang !== 'nexa-followups') return null;
  const raw = message.content.slice(last.position?.start.offset, last.position?.end.offset);
  const lines = raw.trimEnd().split(/\r?\n/);
  if (!/^ {0,3}```nexa-followups\s*$/.test(lines[0]) || !/^ {0,3}```\s*$/.test(lines[lines.length - 1] ?? '')) return null;
  const items = parseFollowupSuggestions(last.value);
  return items.length ? { key: message.id, items } : null;
}
