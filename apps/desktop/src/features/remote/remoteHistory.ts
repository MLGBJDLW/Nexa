interface HistoryMessage {
  id: string;
  content: string;
  totalChars: number;
  sortOrder: number;
}

/** Keep loaded older pages and expanded text when a compact page refreshes. */
export function mergeRemoteHistory<T extends HistoryMessage>(current: T[], incoming: T[]): T[] {
  const firstOrder = incoming[0]?.sortOrder ?? Infinity;
  const byId = new Map(current.map(message => [message.id, message]));
  const older = current.filter(message => message.sortOrder < firstOrder);
  const latest = incoming.map(message => {
    const expanded = byId.get(message.id);
    return expanded && expanded.totalChars === message.totalChars && expanded.content.startsWith(message.content)
      ? expanded : message;
  });
  return [...older, ...latest];
}
