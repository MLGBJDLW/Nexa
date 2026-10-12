/** The single executable format contract used by Nexa and the skill validator.
 * Data only: no URLs, event handlers, tool names, CSS, or arbitrary components. */
export type UiCell = string | number | null;
export type UiBlock =
  | { type: 'text'; text: string }
  | { type: 'metrics'; items: { label: string; value: string | number; detail?: string }[] }
  | { type: 'table'; title?: string; columns: string[]; rows: UiCell[][] }
  | { type: 'bars'; title?: string; unit?: string; items: { label: string; value: number }[] }
  | { type: 'steps'; items: { title: string; detail?: string; status?: 'pending' | 'current' | 'complete' }[] }
  | { type: 'tabs'; tabs: { label: string; blocks: UiBlock[] }[] };
export interface ChatUiDocument { version: 1; title: string; summary?: string; blocks: UiBlock[] }
export const MAX_UI_BYTES = 512 * 1024;
export type UiParseResult = { document: ChatUiDocument; partial: boolean; error?: never }
  | { document?: never; partial: boolean; error: string };

function object(value: unknown, path: string, keys: string[]): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${path}: expected an object`);
  for (const key of Object.keys(value)) if (!keys.includes(key)) throw new Error(`${path}.${key}: unsupported field`);
  return value as Record<string, unknown>;
}
function text(value: unknown, path: string): asserts value is string {
  if (typeof value !== 'string' || value.length > 16000) throw new Error(`${path}: expected text up to 16000 characters`);
}
function list(value: unknown, path: string, max: number): unknown[] {
  if (!Array.isArray(value) || value.length > max) throw new Error(`${path}: expected an array with at most ${max} entries`);
  return value;
}
function cell(value: unknown, path: string): void {
  if (value === null || typeof value === 'number' && Number.isFinite(value)) return;
  text(value, path);
}
function optionalText(value: unknown, path: string): void { if (value !== undefined) text(value, path); }

export function validateChatUi(value: unknown): ChatUiDocument {
  const root = object(value, '$', ['version', 'title', 'summary', 'blocks']);
  if (root.version !== 1) throw new Error('$.version: expected 1');
  text(root.title, '$.title'); optionalText(root.summary, '$.summary');
  let count = 0;
  function blocks(raw: unknown, path: string, depth: number): void {
    if (depth > 4) throw new Error(`${path}: nested UI exceeds 4 levels`);
    for (const [index, entry] of list(raw, path, 64).entries()) {
      const p = `${path}[${index}]`;
      if (++count > 128) throw new Error('UI exceeds 128 blocks');
      const kind = (entry as { type?: unknown } | null)?.type;
      const keys: Record<string, string[]> = { text: ['type','text'], metrics:['type','items'],
        table:['type','title','columns','rows'], bars:['type','title','unit','items'], steps:['type','items'], tabs:['type','tabs'] };
      if (typeof kind !== 'string' || !Object.prototype.hasOwnProperty.call(keys, kind)) throw new Error(`${p}.type: expected text, metrics, table, bars, steps, or tabs`);
      const b = object(entry, p, keys[kind]);
      if (kind === 'text') text(b.text, `${p}.text`);
      if (kind === 'table') {
        optionalText(b.title, `${p}.title`);
        const columns = list(b.columns, `${p}.columns`, 20);
        if (!columns.length) throw new Error(`${p}.columns: at least one column is required`);
        columns.forEach((label, i) => text(label, `${p}.columns[${i}]`));
        const rows = list(b.rows, `${p}.rows`, 200);
        if (rows.length * columns.length > 2000) throw new Error(`${p}: exceeds 2000 table cells; use a spreadsheet for the full dataset`);
        rows.forEach((row, i) => {
          const cells = list(row, `${p}.rows[${i}]`, columns.length);
          if (cells.length !== columns.length) throw new Error(`${p}.rows[${i}]: column count mismatch`);
          cells.forEach((value, j) => cell(value, `${p}.rows[${i}][${j}]`));
        });
      }
      if (kind === 'metrics' || kind === 'bars') {
        if (kind === 'bars') { optionalText(b.title, `${p}.title`); optionalText(b.unit, `${p}.unit`); }
        list(b.items, `${p}.items`, kind === 'metrics' ? 12 : 60).forEach((item, i) => {
          const ip = `${p}.items[${i}]`;
          const v = object(item, ip, kind === 'metrics' ? ['label','value','detail'] : ['label','value']);
          text(v.label, `${ip}.label`);
          if (kind === 'bars' && (typeof v.value !== 'number' || !Number.isFinite(v.value))) throw new Error(`${ip}.value: expected a finite number`);
          if (kind === 'metrics') { if (v.value === null) throw new Error(`${ip}.value: use a number or text`); cell(v.value, `${ip}.value`); optionalText(v.detail, `${ip}.detail`); }
        });
      }
      if (kind === 'steps') list(b.items, `${p}.items`, 60).forEach((item, i) => {
        const ip = `${p}.items[${i}]`; const v = object(item, ip, ['title','detail','status']);
        text(v.title, `${ip}.title`); optionalText(v.detail, `${ip}.detail`);
        if (v.status !== undefined && !['pending','current','complete'].includes(String(v.status))) throw new Error(`${ip}.status: expected pending, current, or complete`);
      });
      if (kind === 'tabs') {
        const tabs = list(b.tabs, `${p}.tabs`, 12);
        if (!tabs.length) throw new Error(`${p}.tabs: at least one tab is required`);
        tabs.forEach((tab, i) => {
          const tp = `${p}.tabs[${i}]`; const v = object(tab, tp, ['label','blocks']);
          text(v.label, `${tp}.label`); blocks(v.blocks, `${tp}.blocks`, depth + 1);
        });
      }
    }
  }
  blocks(root.blocks, '$.blocks', 0);
  return value as ChatUiDocument;
}

/** Recover only fully closed root block objects while a fence is streaming.
 * Never repair final JSON, execute partial scripts, or guess incomplete values. */
function completedPrefix(source: string): string | undefined {
  const stack: string[] = []; let string = false; let escaped = false; let end = -1;
  for (let i = 0; i < source.length; i++) {
    const ch = source[i];
    if (string) { if (escaped) escaped = false; else if (ch === '\\') escaped = true; else if (ch === '"') string = false; continue; }
    if (ch === '"') { string = true; continue; }
    if (ch === '{' || ch === '[') stack.push(ch);
    if (ch === '}' || ch === ']') {
      const opening = stack.pop();
      if (opening !== (ch === '}' ? '{' : '[')) return undefined;
      if (ch === '}' && stack.length === 2 && stack[0] === '{' && stack[1] === '[') end = i + 1;
    }
  }
  return end < 0 ? undefined : source.slice(0, end) + ']}';
}

export function parseChatUi(source: string, streaming = false): UiParseResult {
  if (source.length > MAX_UI_BYTES || new TextEncoder().encode(source).length > MAX_UI_BYTES) return { partial: false, error: 'UI exceeds 512 KiB' };
  try { return { document: validateChatUi(JSON.parse(source)), partial: false }; }
  catch (error) {
    if (streaming) {
      const prefix = completedPrefix(source);
      if (prefix) { try { return { document: validateChatUi(JSON.parse(prefix)), partial: true }; } catch { /* Wait for a complete valid block. */ } }
    }
    return { partial: streaming, error: error instanceof Error ? error.message : 'Invalid UI document' };
  }
}
