"use client";

import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import {
  friendlyErrorMessage,
  items as itemsApi,
  type ItemDetail,
  type ListedItem,
} from "@/lib/chimpflix-api";

/// Render `Title (Year)` when the year is known, otherwise just the
/// title. Used in the inline confirm pane where space is tight and
/// the year disambiguates between same-named items.
function fmtTitle(it: { title: string; year: number | null }): string {
  return it.year != null ? `${it.title} (${it.year})` : it.title;
}

/// Owner-only dialog for merging duplicate items. The current item is
/// the source; the user picks a target from items in the same library
/// and kind. On confirm, every media file (or episode-attached file)
/// is re-pointed onto the target and the source row is deleted.
///
/// Typical trigger: TMDB enrichment renamed a show after initial scan,
/// breaking sort_title-based dedup, leaving two rows for one folder.
export function MergeIntoDialog({
  detail,
  onClose,
  onMerged,
}: {
  detail: ItemDetail;
  onClose: () => void;
  onMerged: (target: ItemDetail) => void;
}) {
  const [query, setQuery] = useState(detail.title);
  const [candidates, setCandidates] = useState<ListedItem[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [merging, setMerging] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  /// Two-step flow: clicking a candidate puts it into `pending` so the
  /// dialog swaps the candidate list for an in-app confirmation
  /// screen. Avoids a native `window.confirm` modal stacked on top of
  /// this one, which looked out of place and used the browser's UA
  /// styling.
  const [pending, setPending] = useState<ListedItem | null>(null);

  useEffect(() => {
    const onEsc = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onEsc);
    return () => window.removeEventListener("keydown", onEsc);
  }, [onClose]);

  // Initial search uses the item's own title so the operator sees
  // likely-duplicate candidates immediately (same library + same kind +
  // a fuzzy title match against the source).
  useEffect(() => {
    void runSearch(detail.title);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function runSearch(q: string) {
    if (!q.trim()) return;
    setSearching(true);
    setError(null);
    try {
      const page = await itemsApi.list({
        library_id: detail.library_id,
        kind: detail.kind,
        q: q.trim(),
        page_size: 25,
      });
      // Exclude the source item itself — merging-into-self is a
      // server-side 400 anyway but the UI shouldn't even offer it.
      setCandidates(page.items.filter((c) => c.id !== detail.id));
    } catch (e) {
      setError(friendlyErrorMessage(e));
      setCandidates(null);
    } finally {
      setSearching(false);
    }
  }

  async function executeMerge(c: ListedItem) {
    if (merging != null) return;
    setMerging(c.id);
    setError(null);
    try {
      const res = await itemsApi.mergeInto(detail.id, c.id);
      onMerged(res.target);
      onClose();
    } catch (e) {
      setError(friendlyErrorMessage(e));
      // Surface the error back on the candidate list so the operator
      // can pick a different target or close — keep `pending` so the
      // confirm pane stays in place when the failure is transient
      // (DB busy, etc.) and they want to retry.
      setPending(null);
    } finally {
      setMerging(null);
    }
  }

  function submit(e: React.FormEvent) {
    e.preventDefault();
    void runSearch(query);
  }

  if (typeof document === "undefined") return null;
  return createPortal(
    <div
      className="fixed inset-0 z-60 flex items-center justify-center bg-black/70 p-4 zf-modal-backdrop"
      onClick={onClose}
    >
      <div
        className="zf-modal-in w-full max-w-3xl overflow-hidden rounded-lg border border-white/10 bg-(--color-surface) shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-white/10 px-6 py-4">
          <div>
            <h2 className="text-lg font-semibold">Merge into…</h2>
            <p className="mt-0.5 text-xs text-white/55">
              Move files from <span className="text-white/85">{detail.title}</span>
              {detail.year != null && ` (${detail.year})`} into another item in
              the same library. The source is deleted afterward.
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
            placeholder="Title of the target item…"
            className="flex-1 rounded bg-black/40 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
          />
          <button
            type="submit"
            disabled={searching || !query.trim()}
            className="rounded bg-(--color-accent) px-4 py-2 text-sm font-semibold text-white transition disabled:opacity-50"
          >
            {searching ? "Searching…" : "Search"}
          </button>
        </form>

        {error && (
          <div className="border-b border-red-500/30 bg-red-500/10 px-6 py-3 text-xs text-red-300">
            {error}
          </div>
        )}

        {pending ? (
          <div className="px-6 py-5">
            <div className="rounded-lg border border-red-500/30 bg-red-500/5 p-4">
              <h3 className="text-sm font-semibold text-red-200">
                Merge {fmtTitle(detail)} INTO {fmtTitle(pending)}?
              </h3>
              <ul className="mt-3 space-y-1.5 text-sm text-white/75">
                <li>
                  • All media files attached to{" "}
                  <span className="text-white">{detail.title}</span> will be
                  re-pointed onto{" "}
                  <span className="text-white">{pending.title}</span>.
                </li>
                <li>
                  • The <span className="text-white">{detail.title}</span> item
                  row will then be deleted.
                </li>
                <li>
                  • Per-user state (watched, my-list, ratings) on the source is
                  lost; the target keeps its own.
                </li>
              </ul>
              <p className="mt-3 text-xs text-white/55">
                This is irreversible — only the file pointers are preserved.
              </p>
            </div>
            <div className="mt-4 flex items-center justify-end gap-2">
              <button
                type="button"
                onClick={() => setPending(null)}
                disabled={merging != null}
                className="rounded-md border border-white/20 px-4 py-2 text-sm font-medium text-white/80 transition-colors hover:border-white/40 hover:text-white disabled:opacity-50"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => void executeMerge(pending)}
                disabled={merging != null}
                className="rounded-md border border-red-500/40 bg-red-500/15 px-4 py-2 text-sm font-semibold text-red-200 transition-colors hover:border-red-500/65 hover:bg-red-500/25 disabled:opacity-50"
              >
                {merging != null ? "Merging…" : "Merge & delete source"}
              </button>
            </div>
          </div>
        ) : (
          <ul className="max-h-[60vh] overflow-y-auto">
            {candidates == null && searching && (
              <li className="px-6 py-8 text-center text-sm text-white/55">
                Searching…
              </li>
            )}
            {candidates != null && candidates.length === 0 && !searching && (
              <li className="px-6 py-8 text-center text-sm text-white/55">
                No other items in this library match. Try a different title.
              </li>
            )}
            {candidates?.map((c) => (
              <li
                key={c.id}
                className="border-b border-white/5 last:border-b-0"
              >
                <button
                  type="button"
                  onClick={() => setPending(c)}
                  disabled={merging != null}
                  className="flex w-full items-start gap-4 px-6 py-4 text-left transition-colors hover:bg-white/5 disabled:opacity-50"
                >
                  <div className="h-24 w-16 shrink-0 overflow-hidden rounded bg-black/50">
                    {c.poster_path ? (
                      // eslint-disable-next-line @next/next/no-img-element
                      <img
                        src={c.poster_path}
                        alt=""
                        loading="lazy"
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
                      <span>#{c.id}</span>
                    </div>
                  </div>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>,
    document.body,
  );
}
