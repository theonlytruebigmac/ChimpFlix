"use client";

import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import Link from "next/link";
import {
  admin as adminApi,
  type LibraryHealthCategory,
  type LibraryHealthItemRow,
  type LibraryHealthResponse,
} from "@/lib/chimpflix-api";

interface CounterTile {
  category: LibraryHealthCategory;
  label: string;
  value: number;
  /// Drives the drill-in modal's title + the hint copy at the top of
  /// the list so each pathology has its own short explanation.
  hint: string;
}

export function AdminLibraryHealthClient({
  report,
}: {
  report: LibraryHealthResponse;
}) {
  const tiles: CounterTile[] = [
    {
      category: "no_files",
      label: "Items without files",
      value: report.items_without_files,
      hint:
        "Movie items whose media_files row was removed (e.g. the file was unmounted or deleted). Re-scan the library, or delete the item if the source is gone for good.",
    },
    {
      category: "no_metadata",
      label: "Items missing every metadata id",
      value: report.items_without_metadata,
      hint:
        "Items the scanner couldn't match to TMDB/IMDb/TVDB/AniList. Open the item modal and use Fix Match to point it at the right entry.",
    },
    {
      category: "no_poster",
      label: "Items without a poster",
      value: report.items_without_poster,
      hint:
        "Items with no poster image cached. Usually metadata-related — Fix Match or Refresh Metadata typically resolves these.",
    },
    {
      category: "no_backdrop",
      label: "Items without a backdrop",
      value: report.items_without_backdrop,
      hint:
        "Items with no backdrop image cached. Same fix as missing posters; some niche titles genuinely don't ship a backdrop on TMDB.",
    },
    {
      category: "orphan_episodes",
      label: "Orphan episodes (no file)",
      value: report.orphan_episodes,
      hint:
        "Episodes that exist in metadata but have no media file on disk. Either the file is missing or the scanner didn't bind it — re-scan the show, or accept the gap.",
    },
    {
      category: "orphan_media_files",
      label: "Orphan media files (no item/episode)",
      value: report.orphan_media_files,
      hint:
        "Files the scanner couldn't bind to any item or episode. Usually a naming mismatch — rename the file to match the show's folder structure and re-scan.",
    },
  ];

  const [drill, setDrill] = useState<CounterTile | null>(null);

  return (
    <div className="space-y-6">
      <section className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {tiles.map((c) => (
          <button
            key={c.category}
            type="button"
            onClick={() => c.value > 0 && setDrill(c)}
            disabled={c.value === 0}
            className={`rounded-lg border p-4 text-left transition-colors ${
              c.value === 0
                ? "cursor-default border-white/10 bg-white/2"
                : "cursor-pointer border-white/10 bg-white/2 hover:border-white/25 hover:bg-white/5"
            }`}
          >
            <div className="flex items-baseline justify-between gap-2">
              <span className="text-xs uppercase tracking-wider text-white/50">
                {c.label}
              </span>
              {c.value > 0 && (
                <span className="text-[10px] uppercase tracking-wider text-white/35">
                  click →
                </span>
              )}
            </div>
            <div
              className={`mt-2 text-3xl font-bold ${
                c.value === 0 ? "text-emerald-300" : "text-amber-300"
              }`}
            >
              {c.value.toLocaleString()}
            </div>
          </button>
        ))}
      </section>

      <section className="rounded-lg border border-white/10 bg-white/2 p-5">
        <h2 className="text-lg font-semibold">
          Missing files on disk ({report.missing_files.length})
        </h2>
        <p className="mt-1 text-xs text-white/50">
          Sample of media file rows whose path no longer exists. The full
          scrub-and-fix workflow belongs to a maintenance task; this is a
          preview limited to the 50 most recent matches.
        </p>
        {report.missing_files.length === 0 ? (
          <p className="mt-3 text-sm text-emerald-300">
            No missing files detected in the sample.
          </p>
        ) : (
          <ul className="mt-3 space-y-1 text-xs">
            {report.missing_files.map((m) => (
              <li
                key={m.id}
                className="flex items-baseline gap-3 border-b border-white/5 py-1.5 last:border-b-0"
              >
                <span className="w-12 shrink-0 font-mono text-white/40">
                  #{m.id}
                </span>
                <span className="min-w-0 flex-1">
                  <div className="truncate font-mono">{m.path}</div>
                  {(m.item_title || m.episode_title) && (
                    <div className="truncate text-white/55">
                      {m.item_title}
                      {m.episode_title ? ` · ${m.episode_title}` : ""}
                    </div>
                  )}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>

      {report.libraries_without_paths.length > 0 && (
        <section className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-5">
          <h2 className="text-lg font-semibold text-amber-100">
            Libraries with no paths
          </h2>
          <p className="mt-1 text-xs text-amber-100/80">
            These libraries exist but have nothing to scan. Add a path
            under Settings → Libraries.
          </p>
          <ul className="mt-3 space-y-1 text-sm text-amber-100">
            {report.libraries_without_paths.map((l) => (
              <li key={l.id}>
                {l.name} <span className="text-amber-100/60">#{l.id}</span>
              </li>
            ))}
          </ul>
        </section>
      )}

      {drill && (
        <HealthDrillIn
          tile={drill}
          onClose={() => setDrill(null)}
        />
      )}
    </div>
  );
}

// ─── Drill-in modal ───────────────────────────────────────────────────────
//
// Fetches the rows behind the clicked counter, paginates 100 at a time,
// and renders each row with a click-through to the title modal where
// applicable (movie items / show items via parent-show lookup for
// orphan episodes). Portal'd to document.body — consistent with the
// other admin modals.

function HealthDrillIn({
  tile,
  onClose,
}: {
  tile: CounterTile;
  onClose: () => void;
}) {
  const [rows, setRows] = useState<LibraryHealthItemRow[] | null>(null);
  const [total, setTotal] = useState<number | null>(null);
  const [offset, setOffset] = useState(0);
  // Start true so the first paint shows the loading state without the
  // effect having to flip it on mount (which trips the
  // react-hooks/set-state-in-effect lint rule). The load-more button
  // sets it true alongside the offset bump.
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const pageSize = 100;

  // Initial load + load-more. Append on subsequent fetches so the
  // operator can scroll back up to earlier matches.
  useEffect(() => {
    let cancelled = false;
    adminApi
      .libraryHealthItems(tile.category, { limit: pageSize, offset })
      .then((r) => {
        if (cancelled) return;
        setRows((prev) => (offset === 0 ? r.rows : [...(prev ?? []), ...r.rows]));
        setTotal(r.total);
        setError(null);
      })
      .catch((e) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [tile.category, offset]);

  function loadMore() {
    setBusy(true);
    setOffset(rows?.length ?? 0);
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  if (typeof document === "undefined") return null;

  const loaded = rows?.length ?? 0;
  const hasMore = total != null && loaded < total;

  return createPortal(
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-60 flex items-center justify-center bg-black/70 p-4"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex max-h-[85vh] w-full max-w-4xl flex-col overflow-hidden rounded-lg border border-white/15 bg-(--color-surface) shadow-2xl">
        <div className="flex items-baseline justify-between gap-3 border-b border-white/10 px-6 py-4">
          <div>
            <h2 className="text-lg font-semibold">{tile.label}</h2>
            <p className="mt-0.5 text-xs text-white/55">
              {total == null
                ? "Loading…"
                : `${total.toLocaleString()} ${total === 1 ? "item" : "items"} affected`}
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
        <div className="border-b border-white/10 bg-black/30 px-6 py-3 text-xs text-white/65">
          {tile.hint}
        </div>
        <div className="flex-1 overflow-y-auto">
          {error && (
            <div className="px-6 py-3 text-xs text-red-300">{error}</div>
          )}
          {rows == null && !error ? (
            <div className="px-6 py-8 text-center text-sm text-white/55">
              Loading…
            </div>
          ) : rows && rows.length === 0 ? (
            <div className="px-6 py-8 text-center text-sm text-white/55">
              Nothing to show — all clear.
            </div>
          ) : (
            <ul className="divide-y divide-white/5">
              {rows?.map((r) => (
                <HealthRow key={`${r.kind}-${r.id}`} row={r} />
              ))}
            </ul>
          )}
        </div>
        <div className="flex items-center justify-between gap-3 border-t border-white/10 px-6 py-3 text-xs text-white/55">
          <span>
            Showing {loaded.toLocaleString()}
            {total != null && total > loaded ? ` of ${total.toLocaleString()}` : ""}
          </span>
          {hasMore && (
            <button
              type="button"
              onClick={loadMore}
              disabled={busy}
              className="rounded border border-white/15 px-3 py-1 text-white/80 hover:bg-white/5 disabled:opacity-50"
            >
              {busy ? "Loading…" : `Load next ${pageSize}`}
            </button>
          )}
        </div>
      </div>
    </div>,
    document.body,
  );
}

function HealthRow({ row }: { row: LibraryHealthItemRow }) {
  const inner = (
    <>
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium text-white/95">
          {row.title}
        </div>
        {row.subtitle && (
          <div className="truncate font-mono text-[11px] text-white/45">
            {row.subtitle}
          </div>
        )}
      </div>
      <div className="shrink-0 text-right text-[11px] text-white/45">
        {row.library_name ?? "—"}
      </div>
    </>
  );

  if (row.item_id_for_modal != null) {
    // Open the title modal in the main app — uses the same
    // `?modal=<id>` URL trick the rails use so the operator can
    // jump straight to the offending item without leaving the
    // admin shell as a "lost" tab.
    return (
      <li>
        <Link
          href={`/?modal=${row.item_id_for_modal}`}
          target="_blank"
          rel="noopener"
          className="flex items-baseline justify-between gap-3 px-6 py-2.5 transition-colors hover:bg-white/5"
        >
          {inner}
        </Link>
      </li>
    );
  }

  // Orphan media file — no parent to open. Show static row with a
  // visual cue that there's no action available.
  return (
    <li className="flex items-baseline justify-between gap-3 px-6 py-2.5 text-white/60">
      {inner}
    </li>
  );
}
