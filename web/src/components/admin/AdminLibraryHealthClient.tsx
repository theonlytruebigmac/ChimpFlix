"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import Link from "next/link";
import {
  admin as adminApi,
  type LibraryHealthCategory,
  type LibraryHealthItemRow,
  type LibraryHealthResponse,
} from "@/lib/chimpflix-api";
import { LoadingPlaceholder } from "../ui/LoadingPlaceholder";

interface CounterTile {
  category: LibraryHealthCategory;
  label: string;
  value: number;
  /// SVG path(s) for the `cf-stat-ico` glyph.
  icon: ReactNode;
  /// Tone applied when the counter is non-zero (a problem to look at).
  /// Zero always renders green regardless of this.
  tone: "amber" | "blue" | "red";
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
      tone: "amber",
      icon: (
        <>
          <rect x="3" y="4" width="18" height="16" rx="2" />
          <path d="M7 4v16M17 4v16M3 9h4M17 9h4M3 15h4M17 15h4" />
        </>
      ),
      hint:
        "Movie items whose media_files row was removed (e.g. the file was unmounted or deleted). Re-scan the library, or delete the item if the source is gone for good.",
    },
    {
      category: "no_metadata",
      label: "Items missing every metadata id",
      value: report.items_without_metadata,
      tone: "amber",
      icon: (
        <>
          <circle cx="8" cy="14" r="4" />
          <path d="M11 11l9-9M17 5l2 2M14 8l2 2" />
        </>
      ),
      hint:
        "Items the scanner couldn't match to TMDB/IMDb/TVDB/AniList. Open the item modal and use Fix Match to point it at the right entry.",
    },
    {
      category: "no_poster",
      label: "Items without a poster",
      value: report.items_without_poster,
      tone: "amber",
      icon: (
        <>
          <rect x="4" y="3" width="16" height="18" rx="2" />
          <path d="M4 15l4-4 4 4 3-3 5 5" />
        </>
      ),
      hint:
        "Items with no poster image cached. Usually metadata-related — Fix Match or Refresh Metadata typically resolves these.",
    },
    {
      category: "no_backdrop",
      label: "Items without a backdrop",
      value: report.items_without_backdrop,
      tone: "blue",
      icon: (
        <>
          <rect x="3" y="5" width="18" height="14" rx="2" />
          <path d="M3 15l5-5 4 4 3-3 6 6" />
          <circle cx="8" cy="9" r="1.5" />
        </>
      ),
      hint:
        "Items with no backdrop image cached. Same fix as missing posters; some niche titles genuinely don't ship a backdrop on TMDB.",
    },
    {
      category: "orphan_episodes",
      label: "Orphan episodes (no file)",
      value: report.orphan_episodes,
      tone: "amber",
      icon: (
        <>
          <rect x="3" y="4" width="18" height="16" rx="2" />
          <path d="M7 4v16M17 4v16M3 9h4M17 9h4M3 15h4M17 15h4" />
        </>
      ),
      hint:
        "Episodes that exist in metadata but have no media file on disk. Either the file is missing or the scanner didn't bind it — re-scan the show, or accept the gap.",
    },
    {
      category: "orphan_media_files",
      label: "Orphan media files (no item/episode)",
      value: report.orphan_media_files,
      tone: "red",
      icon: (
        <>
          <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
          <path d="M12 17v.5" />
        </>
      ),
      hint:
        "Files the scanner couldn't bind to any item or episode. Usually a naming mismatch — rename the file to match the show's folder structure and re-scan.",
    },
  ];

  const [drill, setDrill] = useState<CounterTile | null>(null);

  return (
    <div className="cf-grid" style={{ gridTemplateColumns: "1fr", gap: 24 }}>
      <div className="cf-grid cf-c3">
        {tiles.map((c) => {
          const tone = c.value === 0 ? "green" : c.tone;
          return (
            <button
              key={c.category}
              type="button"
              onClick={() => c.value > 0 && setDrill(c)}
              disabled={c.value === 0}
              className={`cf-stat cf-tone-${tone}`}
              style={{
                textAlign: "left",
                cursor: c.value === 0 ? "default" : "pointer",
                appearance: "none",
              }}
            >
              <div className="cf-stat-top">
                <span className="cf-stat-ico">
                  <svg
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    {c.icon}
                  </svg>
                </span>
                {c.label}
              </div>
              <div className="cf-stat-val">{c.value.toLocaleString()}</div>
              {c.value > 0 && <div className="cf-stat-meta">view →</div>}
            </button>
          );
        })}
      </div>

      <div className="cf-card" style={{ marginBottom: 0 }}>
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">
              Missing files on disk ({report.missing_files.length})
            </div>
            <div className="cf-sub">
              Sample of media file rows whose path no longer exists. The full
              scrub-and-fix workflow belongs to a maintenance task; this is a
              preview limited to the 50 most recent matches.
            </div>
          </div>
        </div>
        <div className="cf-card-body cf-pad">
          {report.missing_files.length === 0 ? (
            <p style={{ color: "var(--ok)", fontSize: 13 }}>
              No missing files detected in the sample.
            </p>
          ) : (
            <ul style={{ display: "flex", flexDirection: "column", gap: 0 }}>
              {report.missing_files.map((m) => (
                <li
                  key={m.id}
                  className="cf-flex cf-gap12"
                  style={{
                    alignItems: "baseline",
                    padding: "6px 0",
                    borderBottom: "1px solid var(--line-faint)",
                    fontSize: 12,
                  }}
                >
                  <span
                    className="cf-mono cf-faint"
                    style={{ width: 48, flex: "none" }}
                  >
                    #{m.id}
                  </span>
                  <span style={{ minWidth: 0, flex: 1 }}>
                    <div
                      className="cf-mono"
                      style={{
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                      }}
                    >
                      {m.path}
                    </div>
                    {(m.item_title || m.episode_title) && (
                      <div
                        className="cf-muted"
                        style={{
                          overflow: "hidden",
                          textOverflow: "ellipsis",
                          whiteSpace: "nowrap",
                        }}
                      >
                        {m.item_title}
                        {m.episode_title ? ` · ${m.episode_title}` : ""}
                      </div>
                    )}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      {report.libraries_without_paths.length > 0 && (
        <div className="cf-banner cf-warn" style={{ marginBottom: 0 }}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M12 3l9 16H3z" />
            <path d="M12 10v4M12 17v.5" />
          </svg>
          <div>
            <b>Libraries with no paths.</b> These libraries exist but have
            nothing to scan. Add a path under Settings → Libraries.
            <ul style={{ marginTop: 8, listStyle: "none", padding: 0 }}>
              {report.libraries_without_paths.map((l) => (
                <li key={l.id}>
                  {l.name} <span className="cf-faint">#{l.id}</span>
                </li>
              ))}
            </ul>
          </div>
        </div>
      )}

      {drill && <HealthDrillIn tile={drill} onClose={() => setDrill(null)} />}
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
  const dialogRef = useRef<HTMLDivElement>(null);
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
        setRows((prev) => (offset === 0 ? r.items : [...(prev ?? []), ...r.items]));
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

  // Move focus into the dialog on mount so keyboard/screen-reader users
  // don't land at an invisible location outside the portal.
  useEffect(() => {
    dialogRef.current?.focus();
  }, []);

  if (typeof document === "undefined") return null;

  const loaded = rows?.length ?? 0;
  const hasMore = total != null && loaded < total;

  return createPortal(
    <div
      ref={dialogRef}
      role="dialog"
      aria-modal="true"
      aria-labelledby="health-drill-title"
      tabIndex={-1}
      className="fixed inset-0 z-60 flex items-center justify-center bg-black/70 p-4"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex max-h-[85vh] w-full max-w-4xl flex-col overflow-hidden rounded-lg border border-white/15 bg-(--color-surface) shadow-2xl">
        <div className="flex items-baseline justify-between gap-3 border-b border-white/10 px-6 py-4">
          <div>
            <h2 id="health-drill-title" className="text-lg font-semibold">{tile.label}</h2>
            <p className="mt-0.5 text-xs text-white/55">
              {total == null ? (
                <LoadingPlaceholder variant="inline" />
              ) : (
                `${total.toLocaleString()} ${total === 1 ? "item" : "items"} affected`
              )}
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
            <LoadingPlaceholder />
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
    // Open the title modal in the main app — uses the `?title=<id>`
    // URL parameter that `ModalRoot` listens for. (Earlier versions
    // of this link used `?modal=` which silently no-op'd because
    // ModalRoot reads `title` exclusively. Always grep ModalRoot's
    // TITLE_PARAM before duplicating this pattern.)
    return (
      <li>
        <Link
          href={`/?title=${row.item_id_for_modal}`}
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
