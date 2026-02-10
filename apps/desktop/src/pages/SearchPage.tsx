import { useState, useEffect, useCallback } from "react";
import * as api from "../lib/api";
import type { SearchResult, QueryLog } from "../types";
import { EvidenceCardComponent } from "../components/EvidenceCard";
import { LoadingSpinner } from "../components/LoadingSpinner";

export function SearchPage() {
  const [query, setQuery] = useState("");
  const [result, setResult] = useState<SearchResult | null>(null);
  const [recentQueries, setRecentQueries] = useState<QueryLog[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
    try {
      const res = await api.search(q.trim());
      setResult(res);
      loadRecentQueries();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
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
      {/* Search input */}
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
            </span>
            <span>{result.searchTimeMs}ms</span>
          </div>

          <div className="space-y-3">
            {result.evidenceCards.map((card) => (
              <EvidenceCardComponent
                key={card.chunkId}
                card={card}
                onSaveToPlaybook={handleSaveToPlaybook}
              />
            ))}
          </div>

          {result.evidenceCards.length === 0 && (
            <p className="py-12 text-center text-sm text-gray-500">
              No results found for &ldquo;{result.query}&rdquo;
            </p>
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
