import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { ChevronRight, Search, X } from 'lucide-react';
import './conversationHistorySearch.css';
import { useTranslation } from '../../i18n';

export interface HistoryHit {
  id: string;
  title: string;
  messageId: string | null;
  messageOrder: number | null;
  snippet: string | null;
}
export interface HistoryPage { items: HistoryHit[]; next: [string, string] | null }
export type FindHistory = (query: string, before: HistoryPage['next']) => Promise<HistoryPage>;

export function ConversationHistorySearch({ find, onPick, archived = false, children }: {
  find: FindHistory; onPick(hit: HistoryHit): void; archived?: boolean; children: ReactNode;
}) {
  const { t } = useTranslation();
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState('');
  const query = draft.trim();
  const [page, setPage] = useState<HistoryPage | null>(null);
  const [error, setError] = useState('');
  const [pending, setPending] = useState(false);
  const [revision, retry] = useState(0);
  const generation = useRef(0);
  const loadingMore = useRef(false);
  const reads = useRef(Promise.resolve());
  useEffect(() => {
    const current = ++generation.current;
    loadingMore.current = false;
    setPage(null);
    setError('');
    setPending(!!query);
    if (!query) return;
    const timer = window.setTimeout(() => {
      // Let the active disk search finish and skip superseded queued terms.
      reads.current = reads.current.then(async () => {
        if (generation.current !== current) return;
        const result = await find(query, null);
        if (generation.current === current) setPage(result);
      }).catch(reason => {
        if (generation.current === current) setError(String(reason));
      }).finally(() => {
        if (generation.current === current) setPending(false);
      });
    }, 180);
    return () => { generation.current++; window.clearTimeout(timer); };
  }, [find, query, revision]);

  async function more() {
    if (!page?.next || loadingMore.current) return;
    const current = generation.current;
    loadingMore.current = true;
    setPending(true);
    setError('');
    try {
      const result = await find(query, page.next);
      if (generation.current === current) setPage({ items: [...page.items, ...result.items], next: result.next });
    } catch (reason) {
      if (generation.current === current) setError(String(reason));
    } finally {
      if (generation.current === current) { loadingMore.current = false; setPending(false); }
    }
  }
  useEffect(() => { if (open) input.current?.focus(); }, [open]);
  function close(restoreFocus: boolean) {
    setDraft('');
    setOpen(false);
    if (restoreFocus) trigger.current?.focus();
  }
  const label = t(archived ? 'chat.searchArchivedConversations' : 'chat.searchConversations');
  return <>
    <div className="history-search-bar" data-open={open}>
      <button type="button" ref={trigger} className="history-search-trigger" aria-label={label}
        aria-expanded={open} aria-controls={id} title={label}
        onClick={() => { if (open) close(true); else setOpen(true); }}>
        <Search size={16} aria-hidden="true" />
      </button>
      <div className="history-search-field" inert={!open}>
        <input ref={input} id={id} type="search" aria-label={label} placeholder={label} value={draft}
          onChange={event => setDraft(event.target.value)}
          onKeyDown={event => {
            if (event.key === 'Escape' && !event.nativeEvent.isComposing) { event.stopPropagation(); close(true); }
          }} />
        <button type="button" aria-label={t('common.close')} onClick={() => close(true)}><X size={15}/></button>
      </div>
    </div>
    {query ? <div className="history-search-results" aria-busy={pending}>
      {error && <p role="alert">{error} <button type="button" onClick={() => page?.next ? void more() : retry(revision + 1)}>{t('common.retry')}</button></p>}
      {page?.items.map(hit => <button type="button" className="history-search-hit" key={hit.id}
        onClick={() => { close(false); onPick(hit); }}>
        <span><strong>{hit.title || t('chat.newConversation')}</strong>{hit.snippet && <small>{hit.snippet}</small>}</span>
        <ChevronRight size={14} aria-hidden="true" />
      </button>)}
      {pending && <p role="status">{t('common.loading')}</p>}
      {!pending && !error && page?.items.length === 0 && <p role="status">{t('chat.noSearchResults')}</p>}
      {page?.next && !error && <button type="button" disabled={pending} onClick={() => void more()}>{t('remote.moreChats')}</button>}
    </div> : children}
  </>;
}
