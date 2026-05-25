"use client";

import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  friendlyErrorMessage,
  items as itemsApi,
  type ItemDetail,
  type MatchCandidate,
} from "@/lib/chimpflix-api";
import { useFocusTrap } from "@/lib/use-focus-trap";

const TMDB_IMAGE = "https://image.tmdb.org/t/p/w185";

// Plex-style "Fix Match" dialog: free-text search across TMDB candidates,
// pick the right title, apply it. Re-applying a match clears existing
// metadata + re-runs enrichment with the new tmdb_id.
export function FixMatchDialog({
  detail,
  onClose,
  onApplied,
}: {
  detail: ItemDetail;
  onClose: () => void;
  onApplied: (next: ItemDetail) => void;
}) {
  const [query, setQuery] = useState(detail.title);
  const [year, setYear] = useState<string>(
    detail.year != null ? String(detail.year) : "",
  );
  const [candidates, setCandidates] = useState<MatchCandidate[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [searching, setSearching] = useState(false);
  const [applying, setApplying] = useState<number | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const titleId = "fixmatch-dialog-title";

  // Escape + Tab cycling + restore-focus-on-close handled centrally
  // by the shared hook (was a duplicated `keydown` listener here).
  useFocusTrap(dialogRef, { onClose });

  // Run an initial search so the dialog opens with results already populated
  // for the existing title — matches Plex's UX where Fix Match shows the
  // current best guess up front.
  useEffect(() => {
    void runSearch(detail.title, detail.year ?? undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function runSearch(q: string, y: number | undefined) {
    if (!q.trim()) return;
    setSearching(true);
    setError(null);
    try {
      const res = await itemsApi.matchSearch(detail.id, q.trim(), y);
      setCandidates(res.candidates);
    } catch (e) {
      setError(friendlyErrorMessage(e));
      // Leave candidates as null so the dialog renders only the error
      // banner — setting it to [] would also show "No matches", which
      // is misleading when the real failure is an upstream/server error.
      setCandidates(null);
    } finally {
      setSearching(false);
    }
  }

  async function apply(c: MatchCandidate) {
    if (applying != null) return;
    setApplying(c.tmdb_id);
    setError(null);
    try {
      const next = await itemsApi.matchApply(detail.id, c.tmdb_id);
      onApplied(next);
      onClose();
    } catch (e) {
      setError(friendlyErrorMessage(e));
    } finally {
      setApplying(null);
    }
  }

  function submit(e: React.FormEvent) {
    e.preventDefault();
    const y = year.trim() ? Number.parseInt(year, 10) : undefined;
    void runSearch(query, Number.isFinite(y) ? (y as number) : undefined);
  }

  // Portal to document.body so the dialog escapes the TitleModalShell's
  // `.zf-modal-in` ancestor (transform animation → new containing
  // block for fixed descendants → dialog ends up wherever the
  // possibly-scrolled modal card is, instead of centered in the
  // viewport).
  if (typeof document === "undefined") return null;
  return createPortal(
    <div
      className="fixed inset-0 z-60 flex items-center justify-center bg-black/70 p-4 zf-modal-backdrop"
      onClick={onClose}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="zf-modal-in w-full max-w-3xl overflow-hidden rounded-lg border border-white/10 bg-(--color-surface) shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-white/10 px-6 py-4">
          <div>
            <h2 id={titleId} className="text-lg font-semibold">
              Fix Match
            </h2>
            <p className="mt-0.5 text-xs text-white/55">
              Current: {detail.title}
              {detail.year != null && ` (${detail.year})`}
              {detail.tmdb_id != null && ` · TMDB #${detail.tmdb_id}`}
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="text-white/60 transition-colors hover:text-white"
          >
            ✕
          </button>
        </div>

        <form
          onSubmit={submit}
          className="flex gap-2 border-b border-white/10 px-6 py-3"
        >
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Title to search…"
            className="flex-1 rounded bg-black/40 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
          />
          <input
            type="number"
            value={year}
            onChange={(e) => setYear(e.target.value)}
            placeholder="Year"
            className="w-24 rounded bg-black/40 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
          />
          <button
            type="submit"
            disabled={searching || !query.trim()}
            className="rounded bg-(--color-accent) px-4 py-2 text-sm font-semibold text-white transition disabled:opacity-50"
          >
            {searching ? "Searching…" : "Search"}
          </button>
        </form>

        <ul className="max-h-[60vh] overflow-y-auto">
          {candidates == null && searching && (
            <li className="px-6 py-8 text-center text-sm text-white/55">
              Searching TMDB…
            </li>
          )}
          {candidates != null && candidates.length === 0 && !searching && (
            <li className="px-6 py-8 text-center text-sm text-white/55">
              No matches. Try a different title or year.
            </li>
          )}
          {candidates?.map((c) => {
            const isCurrent = detail.tmdb_id === c.tmdb_id;
            return (
              <li key={c.tmdb_id} className="border-b border-white/5 last:border-b-0">
                <button
                  type="button"
                  onClick={() => apply(c)}
                  disabled={applying != null}
                  className="flex w-full items-start gap-4 px-6 py-4 text-left transition-colors hover:bg-white/5 disabled:opacity-50"
                >
                  <div className="h-24 w-16 shrink-0 overflow-hidden rounded bg-black/50">
                    {c.poster_path ? (
                      // eslint-disable-next-line @next/next/no-img-element
                      <img
                        src={`${TMDB_IMAGE}${c.poster_path}`}
                        alt=""
                        loading="lazy"
                        onError={(e) => {
                          (e.currentTarget as HTMLImageElement).style.display =
                            "none";
                        }}
                        className="h-full w-full object-cover"
                      />
                    ) : (
                      <div className="flex h-full w-full items-center justify-center text-xs text-white/30">
                        No art
                      </div>
                    )}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-baseline justify-between gap-3">
                      <span className="truncate text-base font-semibold">
                        {c.title}
                      </span>
                      <span className="shrink-0 text-sm text-white/55">
                        {c.year ?? "—"}
                      </span>
                    </div>
                    {c.summary && (
                      <p className="mt-1 line-clamp-3 text-sm text-white/75">
                        {c.summary}
                      </p>
                    )}
                    <div className="mt-2 flex items-center gap-2 text-xs text-white/45">
                      <span>TMDB #{c.tmdb_id}</span>
                      {isCurrent && (
                        <span className="rounded bg-(--color-accent)/20 px-1.5 py-0.5 text-(--color-accent)">
                          current
                        </span>
                      )}
                      {applying === c.tmdb_id && (
                        <span className="text-(--color-accent)">applying…</span>
                      )}
                    </div>
                  </div>
                </button>
              </li>
            );
          })}
        </ul>

        {error && (
          <div className="border-t border-(--color-accent)/30 bg-(--color-accent)/10 px-6 py-2 text-sm text-(--color-accent)">
            {error}
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}
