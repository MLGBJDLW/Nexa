import { useState, useEffect, useCallback } from "react";
import * as api from "../lib/api";
import type { SearchResult, QueryLog, Feedback } from "../types";
import { EvidenceCardComponent } from "../components/EvidenceCard";
import { LoadingSpinner } from "../components/LoadingSpinner";

export function SearchPage() {
  const [query, setQuery] = useState("");
  const [result, setResult] = useState<SearchResult | null>(null);
  const [recentQueries, setRecentQueries] = useState<QueryLog[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [searchMode, setSearchMode] = useState<'fts' | 'hybrid'>('fts');
  const [feedbackMap, setFeedbackMap] = useState<Record<string, Feedback>>({}); // chunkId -> Feedback

  // ── Save-to-playbook modal state ──────────────────────────────────────
  const [savingChunkId, setSavingChunkId] = useState<string | null>(null);
  const [playbooks, setPlaybooks] = useState<{ id: string; title: string }[]>([]);
  const [selectedPlaybookId, setSelectedPlaybookId] = useState("");
  const [citationNote, setCitationNote] = useState("");

  const loadRecentQueries = useCallback(async () => {
    try {
      const recent = await api.getRecentQueries(10);
      setRecentQueries(recent);
    } catch {
      // non-critical — ignore
    }
  }, []);

  useEffect(() => {
    loadRecentQueries();
  }, [loadRecentQueries]);

  const handleSearch = async (text?: string) => {
    const q = text ?? query;
    if (!q.trim()) return;
    setLoading(true);
    setError(null);
    setFeedbackMap({});
    try {
      const res = searchMode === 'hybrid'
        ? await api.hybridSearch(q.trim())
        : await api.search(q.trim());
      setResult({ ...res, searchMode });
      loadRecentQueries();
      // Load existing feedback for this query
      try {
        const feedbacks = await api.getFeedbackForQuery(q.trim());
        const map: Record<string, Feedback> = {};
        for (const fb of feedbacks) {
          map[fb.chunkId] = fb;
        }
        setFeedbackMap(map);
      } catch { /* non-critical */ }
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const handleFeedback = async (chunkId: string, action: 'upvote' | 'downvote' | 'pin') => {
    if (!result) return;
    try {
      // If same action already exists, delete it (toggle off)
      const existing = feedbackMap[chunkId];
      if (existing && existing.action === action) {
        await api.deleteFeedback(existing.id);
        setFeedbackMap((prev) => {
          const next = { ...prev };
          delete next[chunkId];
          return next;
        });
        return;
      }
      // Otherwise, if different action exists, delete old then add new
      if (existing) {
        await api.deleteFeedback(existing.id);
      }
      const fb = await api.addFeedback(chunkId, result.query, action);
      setFeedbackMap((prev) => ({ ...prev, [chunkId]: fb }));
    } catch (e) {
      setError(String(e));
    }
  };

  const handleSaveToPlaybook = async (chunkId: string) => {
    setSavingChunkId(chunkId);
    try {
      const pbs = await api.listPlaybooks();
      setPlaybooks(pbs.map((p) => ({ id: p.id, title: p.title })));
      if (pbs.length > 0) setSelectedPlaybookId(pbs[0].id);
    } catch {
      // ignore
    }
  };

  const confirmSave = async () => {
    if (!savingChunkId || !selectedPlaybookId) return;
    try {
      await api.addCitation(selectedPlaybookId, savingChunkId, citationNote, 0);
      setSavingChunkId(null);
      setCitationNote("");
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="mx-auto max-w-3xl p-6">
      {/* Search input + mode toggle */}
      <div className="mb-6">
        <div className="relative">
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleSearch()}
            placeholder="Search your knowledge..."
            className="w-full rounded-lg border border-gray-700 bg-gray-800 px-4 py-3 text-gray-100 placeholder-gray-500 focus:border-transparent focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
          <button
            onClick={() => handleSearch()}
            disabled={loading}
            className="absolute right-2 top-1/2 -translate-y-1/2 rounded-md bg-blue-600 px-3 py-1.5 text-xs font-medium text-white transition hover:bg-blue-500 disabled:opacity-50"
          >
            Search
          </button>
        </div>
        {/* Search mode toggle */}
        <div className="mt-2 flex items-center gap-2">
          <div className="inline-flex rounded-full border border-gray-700 bg-gray-800 p-0.5 text-xs">
            <button
              onClick={() => setSearchMode('fts')}
              className={`rounded-full px-3 py-1 font-medium transition ${
                searchMode === 'fts'
                  ? 'bg-blue-600 text-white'
                  : 'text-gray-400 hover:text-gray-200'
              }`}
            >
              FTS
            </button>
            <button
              onClick={() => setSearchMode('hybrid')}
              className={`rounded-full px-3 py-1 font-medium transition ${
                searchMode === 'hybrid'
                  ? 'bg-blue-600 text-white'
                  : 'text-gray-400 hover:text-gray-200'
              }`}
            >
              Hybrid
            </button>
          </div>
          <span className="text-xs text-gray-500">
            {searchMode === 'hybrid' ? 'Semantic + keyword search' : 'Full-text keyword search'}
          </span>
        </div>
      </div>

      {/* Error */}
      {error && (
        <div className="mb-4 rounded-md border border-red-800 bg-red-900/30 px-4 py-2 text-sm text-red-300">
          {error}
        </div>
      )}

      {/* Loading */}
      {loading && <LoadingSpinner className="py-12" />}

      {/* Results */}
      {result && !loading && (
        <>
          <div className="mb-4 flex items-baseline justify-between text-xs text-gray-500">
            <span>
              {result.totalMatches} match{result.totalMatches !== 1 ? "es" : ""}
              {result.searchMode && (
                <span className="ml-2 rounded-full border border-gray-700 px-2 py-0.5 text-gray-400">
                  {result.searchMode === 'hybrid' ? 'Hybrid' : 'FTS'}
                </span>
              )}
            </span>
            <span>{result.searchTimeMs}ms</span>
          </div>

          {/* Uncertainty banner */}
          {(result.evidenceCards.length < 3 && result.evidenceCards.length > 0 &&
            result.evidenceCards.every((c) => c.score < 1)) && (
            <div className="mb-4 flex items-center gap-2 rounded-lg border border-amber-700/50 bg-amber-900/20 px-4 py-3 text-sm text-amber-300">
              <span className="text-lg">⚠️</span>
              <span>证据不足 — 现有资料中未找到充分证据，请考虑添加更多来源</span>
            </div>
          )}

          <div className="space-y-3">
            {result.evidenceCards.map((card) => (
              <EvidenceCardComponent
                key={card.chunkId}
                card={card}
                onSaveToPlaybook={handleSaveToPlaybook}
                onFeedback={handleFeedback}
                activeFeedback={feedbackMap[card.chunkId]?.action ?? null}
              />
            ))}
          </div>

          {result.evidenceCards.length === 0 && (
            <div className="space-y-3">
              <div className="flex items-center gap-2 rounded-lg border border-amber-700/50 bg-amber-900/20 px-4 py-3 text-sm text-amber-300">
                <span className="text-lg">⚠️</span>
                <span>证据不足 — 现有资料中未找到充分证据，请考虑添加更多来源</span>
              </div>
              <p className="py-8 text-center text-sm text-gray-500">
                No results found for &ldquo;{result.query}&rdquo;
              </p>
            </div>
          )}
        </>
      )}

      {/* Recent queries */}
      {!result && !loading && recentQueries.length > 0 && (
        <div className="mt-8">
          <h3 className="mb-3 text-xs font-medium uppercase tracking-wider text-gray-500">
            Recent Searches
          </h3>
          <div className="space-y-1">
            {recentQueries.map((q) => (
              <button
                key={q.id}
                onClick={() => {
                  setQuery(q.queryText);
                  handleSearch(q.queryText);
                }}
                className="flex w-full items-center justify-between rounded-md px-3 py-2 text-left text-sm text-gray-300 transition hover:bg-gray-800"
              >
                <span className="truncate">{q.queryText}</span>
                <span className="shrink-0 text-xs text-gray-600">
                  {q.resultCount} results
                </span>
              </button>
            ))}
          </div>
        </div>
      )}

      {/* Save to Playbook modal */}
      {savingChunkId && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
          <div className="w-full max-w-sm rounded-lg border border-gray-700 bg-gray-900 p-5">
            <h3 className="mb-4 text-sm font-semibold">Save to Playbook</h3>
            {playbooks.length === 0 ? (
              <p className="text-sm text-gray-400">
                No playbooks yet. Create one on the Playbooks page first.
              </p>
            ) : (
              <>
                <label className="mb-1 block text-xs text-gray-400">Playbook</label>
                <select
                  value={selectedPlaybookId}
                  onChange={(e) => setSelectedPlaybookId(e.target.value)}
                  className="mb-3 w-full rounded border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100"
                >
                  {playbooks.map((pb) => (
                    <option key={pb.id} value={pb.id}>
                      {pb.title}
                    </option>
                  ))}
                </select>

                <label className="mb-1 block text-xs text-gray-400">Note (optional)</label>
                <input
                  type="text"
                  value={citationNote}
                  onChange={(e) => setCitationNote(e.target.value)}
                  className="mb-4 w-full rounded border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100"
                  placeholder="Add a note..."
                />
              </>
            )}
            <div className="flex justify-end gap-2">
              <button
                onClick={() => {
                  setSavingChunkId(null);
                  setCitationNote("");
                }}
                className="rounded px-3 py-1.5 text-xs text-gray-400 hover:text-gray-200"
              >
                Cancel
              </button>
              {playbooks.length > 0 && (
                <button
                  onClick={confirmSave}
                  className="rounded bg-blue-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-blue-500"
                >
                  Save
                </button>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
