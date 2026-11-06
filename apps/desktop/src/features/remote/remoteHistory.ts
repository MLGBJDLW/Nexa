interface HistoryMessage {
  id: string;
  content: string;
  totalChars: number;
  sortOrder: number;
  thinking?: string;
  thinkingTotalChars?: number;
  role?: string;
  intermediate?: boolean;
  continuesTurn?: boolean;
  toolCalls?: readonly unknown[];
}

/** Keep loaded older pages and expanded text when a compact page refreshes. */
export function mergeRemoteHistory<T extends HistoryMessage>(current: T[], incoming: T[]): T[] {
  const firstOrder = incoming[0]?.sortOrder ?? Infinity;
  const byId = new Map(current.map(message => [message.id, message]));
  const older = current.filter(message => message.sortOrder < firstOrder);
  const latest = incoming.map(message => {
    const expanded = byId.get(message.id);
    if (!expanded) return message;
    const content = expanded.totalChars === message.totalChars && expanded.content.startsWith(message.content) ? expanded.content : message.content;
    const thinking = expanded.thinkingTotalChars === message.thinkingTotalChars && expanded.thinking?.startsWith(message.thinking ?? '') ? expanded.thinking : message.thinking;
    if (content === expanded.content && thinking === expanded.thinking
      && message.totalChars === expanded.totalChars && message.thinkingTotalChars === expanded.thinkingTotalChars
      && message.intermediate === expanded.intermediate && message.toolCalls === expanded.toolCalls
      && message.role === expanded.role && message.continuesTurn === expanded.continuesTurn) return expanded;
    if (content === message.content && thinking === message.thinking) return message;
    return { ...message, content, thinking };
  });
  return [...older, ...latest];
}
