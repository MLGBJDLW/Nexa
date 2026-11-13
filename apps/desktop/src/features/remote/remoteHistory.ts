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
  precedingUserId?: string;
  toolCalls?: readonly unknown[];
}

function sameRecord(left: object, right: object): boolean {
  const keys = Object.keys(left);
  const leftFields = left as Record<string, unknown>;
  const rightFields = right as Record<string, unknown>;
  return keys.length === Object.keys(right).length
    && keys.every(key => Object.prototype.hasOwnProperty.call(right, key) && leftFields[key] === rightFields[key]);
}

function sameRecords(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false;
  return left.every((item, index) => item === right[index]
    || (item !== null && typeof item === 'object' && right[index] !== null
      && typeof right[index] === 'object' && sameRecord(item, right[index])));
}

function sameMetadata<T extends HistoryMessage>(left: T, right: T): boolean {
  // Compare all wire metadata, including optional steering and attachments.
  // Fresh JSON arrays with unchanged flat records should not invalidate every
  // rendered turn, but new/removed fields must still reach the transcript.
  const isMetadata = (key: string) => key !== 'id' && key !== 'content' && key !== 'thinking';
  const keys = Object.keys(left).filter(isMetadata) as (keyof T)[];
  return keys.length === Object.keys(right).filter(isMetadata).length
    && keys.every(key => Object.prototype.hasOwnProperty.call(right, key) && (key === 'toolCalls' || key === 'attachments'
      ? sameRecords(left[key], right[key]) : left[key] === right[key]));
}

/** Keep loaded older pages and expanded text when a compact page refreshes. */
export function mergeRemoteHistory<T extends HistoryMessage>(current: T[], incoming: T[]): T[] {
  if (!incoming.length) return current;
  if (!current.length) return incoming;
  const firstOrder = incoming[0]?.sortOrder ?? Infinity;
  const incomingIds = new Set(incoming.map(message => message.id));
  const byId = new Map<string, T>();
  const older: T[] = [];
  for (const message of current) {
    const id = message.id;
    byId.set(id, message);
    if (message.sortOrder < firstOrder && !incomingIds.has(id)) older.push(message);
  }
  const latest = incoming.map(message => {
    const expanded = byId.get(message.id);
    if (!expanded) return message;
    const content = expanded.totalChars === message.totalChars && expanded.content.startsWith(message.content) ? expanded.content : message.content;
    const thinking = expanded.thinkingTotalChars === message.thinkingTotalChars && expanded.thinking?.startsWith(message.thinking ?? '') ? expanded.thinking : message.thinking;
    if (content === expanded.content && thinking === expanded.thinking
      && sameMetadata(message, expanded)) return expanded;
    if (content === message.content && thinking === message.thinking) return message;
    return { ...message, content, thinking };
  });
  const merged = [...older, ...latest];
  return merged.length === current.length && merged.every((message, index) => message === current[index]) ? current : merged;
}
