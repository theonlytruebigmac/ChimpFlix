"use client";

/// Grid-style browse-all surface for a library. Lives at
/// `/library/[id]/browse` next to the Netflix-style rail view at
/// `/library/[id]`. Solves three complementary problems:
///
///   1. "Where's that movie I downloaded?" — every title in the
///      library, paginated, sortable.
///   2. "What haven't I watched yet?" — Status chips filter to
///      unwatched / in-progress / watched.
///   3. "Why isn't my file showing up?" — switch the match-state
///      chip to "Unmatched" and you see the stubs the scanner
///      created for files whose names didn't fit the auto-matcher's
///      regex. From here the operator can open the modal and run
///      Fix Match.

import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useRef, useState, useTransition } from "react";

import { Card } from "@/components/Card";
import { Pagination } from "@/components/ui/Pagination";
import { adaptItem } from "@/lib/chimpflix-adapt";
import type { ItemKind, ItemSort, ListedItem } from "@/lib/chimpflix-api";

type MatchFilter = "all" | "matched" | "unmatched";
type StatusFilter = "all" | "unwatched" | "in_progress" | "watched";
type DecadeFilter =
  | "all"
  | "2020"
  | "2010"
  | "2000"
  | "1990"
  | "1980"
  | "1970"
  | "older";

const SORT_LABEL: Record<ItemSort, string> = {
  recently_added: "Recently added",
  title: "A → Z",
  year_desc: "Year (newest)",
  year_asc: "Year (oldest)",
  rating_desc: "Rating",
  duration_desc: "Longest",
  duration_asc: "Shortest",
  last_played: "Last played",
  random: "Shuffle",
  size_desc: "Largest on disk",
  size_asc: "Smallest on disk",
};

const RESOLUTION_OPTIONS: ReadonlyArray<{ value: string; label: string }> = [
  { value: "4k", label: "4K" },
  { value: "1080", label: "1080p" },
  { value: "720", label: "720p" },
  { value: "sd", label: "SD" },
];
const HDR_OPTIONS: ReadonlyArray<{ value: string; label: string }> = [
  { value: "hdr10", label: "HDR10" },
  { value: "hlg", label: "HLG" },
  { value: "sdr", label: "SDR" },
];
const CODEC_OPTIONS: ReadonlyArray<{ value: string; label: string }> = [
  { value: "hevc", label: "HEVC (H.265)" },
  { value: "h264", label: "H.264" },
  { value: "av1", label: "AV1" },
  { value: "vp9", label: "VP9" },
  { value: "mpeg4", label: "MPEG-4" },
  { value: "mpeg2video", label: "MPEG-2" },
  { value: "other", label: "Other" },
];
const FILTER_OPTION_LABEL: Record<string, string> = Object.fromEntries(
  [...RESOLUTION_OPTIONS, ...HDR_OPTIONS, ...CODEC_OPTIONS].map((o) => [
    o.value,
    o.label,
  ]),
);

const MATCH_LABEL: Record<MatchFilter, string> = {
  all: "All",
  matched: "Auto-matched",
  unmatched: "Unmatched (needs review)",
};

const STATUS_LABEL: Record<StatusFilter, string> = {
  all: "Any status",
  unwatched: "Unwatched",
  in_progress: "In progress",
  watched: "Watched",
};

const DECADE_LABEL: Record<DecadeFilter, string> = {
  all: "Any year",
  "2020": "2020s",
  "2010": "2010s",
  "2000": "2000s",
  "1990": "1990s",
  "1980": "1980s",
  "1970": "1970s",
  older: "Before 1970",
};

interface Props {
  libraryId: number;
  initialItems: ListedItem[];
  initialTotal: number;
  initialPage: number;
  initialSort: ItemSort;
  initialQuery: string;
  initialFilter: MatchFilter;
  initialStatus: StatusFilter;
  initialDecade: DecadeFilter;
  /** Echoes the seed the route page resolved for the current page.
   *  Empty when sort !== "random". */
  initialSeed: number | null;
  /** Per-file attribute filters parsed from the URL. Empty arrays
   *  when not active. */
  initialResolutions: ReadonlyArray<string>;
  initialHdr: ReadonlyArray<string>;
  initialCodecs: ReadonlyArray<string>;
  pageSize: number;
  kind: ItemKind;
}

export function LibraryBrowseClient({
  initialItems,
  initialTotal,
  initialPage,
  initialSort,
  initialQuery,
  initialFilter,
  initialStatus,
  initialDecade,
  initialSeed,
  initialResolutions,
  initialHdr,
  initialCodecs,
  pageSize,
  libraryId,
}: Props) {
  const router = useRouter();
  const searchParams = useSearchParams();
  const [, startTransition] = useTransition();

  // Track query in local state so the input is responsive while
  // we debounce the URL push. Sort/filter update the URL
  // immediately (no debounce needed — they're discrete clicks).
  const [query, setQuery] = useState(initialQuery);

  // Mirror searchParams into a ref so the debounced timer reads the
  // LATEST snapshot at fire time, not the one captured 300 ms ago.
  // Without this, a user who types then quickly clicks a filter chip
  // before the debounce fires has their just-committed filter clobbered
  // by the stale-snapshot URL push.
  const searchParamsRef = useRef(searchParams);
  useEffect(() => {
    searchParamsRef.current = searchParams;
  }, [searchParams]);

  // Re-sync local `query` state when the server-rendered initial value
  // changes. Browser-back / external navigation rewrites the URL —
  // without this, the input box keeps showing the old text.
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setQuery(initialQuery);
  }, [initialQuery]);

  // Debounce URL updates as the user types in the search box.
  useEffect(() => {
    const trimmed = query.trim();
    if (trimmed === initialQuery) return; // no change from server-rendered state
    const t = window.setTimeout(() => {
      const params = new URLSearchParams(
        searchParamsRef.current?.toString() ?? "",
      );
      if (trimmed) params.set("q", trimmed);
      else params.delete("q");
      params.delete("page"); // search change resets paging
      startTransition(() => {
        router.replace(`/library/${libraryId}/browse?${params.toString()}`, {
          scroll: false,
        });
      });
    }, 300);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  const updateParam = useCallback(
    (mutate: (p: URLSearchParams) => void) => {
      const params = new URLSearchParams(searchParams?.toString() ?? "");
      mutate(params);
      startTransition(() => {
        router.replace(`/library/${libraryId}/browse?${params.toString()}`, {
          scroll: false,
        });
      });
    },
    [router, searchParams, libraryId],
  );

  const setSort = (next: ItemSort) =>
    updateParam((p) => {
      p.set("sort", next);
      p.delete("page");
      // Random sort needs a stable seed so paging doesn't reshuffle. Generate
      // a new one when the user picks random (or arrives without one); strip
      // it the moment they switch to any other sort so the URL stays clean
      // and ?seed= doesn't linger as confusing dead state.
      if (next === "random") {
        if (!p.get("seed")) {
          p.set("seed", String(Math.floor(Math.random() * 1_000_000_000)));
        }
      } else {
        p.delete("seed");
      }
    });
  const setFilter = (next: MatchFilter) =>
    updateParam((p) => {
      if (next === "all") p.delete("filter");
      else p.set("filter", next);
      p.delete("page");
    });
  const setStatus = (next: StatusFilter) =>
    updateParam((p) => {
      if (next === "all") p.delete("status");
      else p.set("status", next);
      p.delete("page");
    });
  const setDecade = (next: DecadeFilter) =>
    updateParam((p) => {
      if (next === "all") p.delete("decade");
      else p.set("decade", next);
      p.delete("page");
    });
  const setPage = (next: number) => {
    updateParam((p) => {
      if (next <= 1) p.delete("page");
      else p.set("page", String(next));
    });
    // Page changes are explicit "show me different content" actions, so
    // returning the user to the top of the grid is what they expect —
    // sort/filter changes keep `scroll: false` since the user is anchored.
    if (typeof window !== "undefined") {
      window.scrollTo({ top: 0, behavior: "smooth" });
    }
  };
  const setPageSize = (next: number) =>
    updateParam((p) => {
      // 60 is the route default — omit when matching so the URL stays clean.
      if (next === 60) p.delete("page_size");
      else p.set("page_size", String(next));
      // Resetting page count: the user's current page might exceed the new
      // total after enlarging items-per-page. Cheap to nuke and let them
      // re-land on page 1.
      p.delete("page");
    });

  const reshuffle = () =>
    updateParam((p) => {
      p.set("sort", "random");
      p.set("seed", String(Math.floor(Math.random() * 1_000_000_000)));
      p.delete("page");
    });

  /// Toggle membership of `value` in the CSV-encoded `?key=` parameter.
  /// Empty list strips the param entirely so the URL stays clean and the
  /// server treats "absent" identically to "all".
  const toggleMulti = (
    key: "resolutions" | "hdr" | "codecs",
    value: string,
    current: ReadonlyArray<string>,
  ) =>
    updateParam((p) => {
      const next = current.includes(value)
        ? current.filter((v) => v !== value)
        : [...current, value];
      if (next.length === 0) p.delete(key);
      else p.set(key, next.join(","));
      p.delete("page");
    });

  const clearFileFilters = () =>
    updateParam((p) => {
      p.delete("resolutions");
      p.delete("hdr");
      p.delete("codecs");
      p.delete("page");
    });

  const activeFileFilterCount =
    initialResolutions.length + initialHdr.length + initialCodecs.length;

  /// Wipe every filter (match / status / decade / file filters / search).
  /// Drives the empty-state "Clear all filters" button so a user staring
  /// at "No results" doesn't have to undo five chips by hand.
  const clearAllFilters = () =>
    updateParam((p) => {
      p.delete("filter");
      p.delete("status");
      p.delete("decade");
      p.delete("resolutions");
      p.delete("hdr");
      p.delete("codecs");
      p.delete("q");
      p.delete("page");
    });

  const hasAnyFilter =
    activeFileFilterCount > 0 ||
    initialFilter !== "all" ||
    initialStatus !== "all" ||
    initialDecade !== "all" ||
    Boolean(query);

  const items = initialItems.map(adaptItem);

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <MatchChips current={initialFilter} onChange={setFilter} />
        <StatusChips current={initialStatus} onChange={setStatus} />
        <div className="ml-auto flex flex-wrap items-center gap-2">
          <FiltersPopover
            activeCount={activeFileFilterCount}
            resolutions={initialResolutions}
            hdr={initialHdr}
            codecs={initialCodecs}
            onToggle={toggleMulti}
            onClear={clearFileFilters}
          />
          <label className="flex items-center gap-2 text-[12px] text-white/65">
            <span>Decade</span>
            <select
              value={initialDecade}
              onChange={(e) => setDecade(e.target.value as DecadeFilter)}
              className="rounded-md border border-white/15 bg-black/40 px-2 py-1 text-[12.5px] text-white/90 focus:border-white/35 focus:outline-none"
            >
              {(Object.keys(DECADE_LABEL) as DecadeFilter[]).map((d) => (
                <option key={d} value={d}>
                  {DECADE_LABEL[d]}
                </option>
              ))}
            </select>
          </label>
          <label className="flex items-center gap-2 text-[12px] text-white/65">
            <span>Sort</span>
            <select
              value={initialSort}
              onChange={(e) => setSort(e.target.value as ItemSort)}
              className="rounded-md border border-white/15 bg-black/40 px-2 py-1 text-[12.5px] text-white/90 focus:border-white/35 focus:outline-none"
            >
              {(Object.keys(SORT_LABEL) as ItemSort[]).map((s) => (
                <option key={s} value={s}>
                  {SORT_LABEL[s]}
                </option>
              ))}
            </select>
          </label>
          {initialSort === "random" && initialSeed !== null && (
            <button
              type="button"
              onClick={reshuffle}
              title="Reshuffle"
              className="inline-flex items-center gap-1 rounded-md border border-white/15 bg-black/40 px-2.5 py-1 text-[12px] text-white/80 transition-colors hover:border-white/35 hover:text-white"
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
                <polyline points="16 3 21 3 21 8" />
                <line x1="4" y1="20" x2="21" y2="3" />
                <polyline points="21 16 21 21 16 21" />
                <line x1="15" y1="15" x2="21" y2="21" />
                <line x1="4" y1="4" x2="9" y2="9" />
              </svg>
              <span>Reshuffle</span>
            </button>
          )}
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search in this library"
            className="w-48 rounded-md border border-white/15 bg-black/40 px-3 py-1.5 text-[12.5px] text-white/90 placeholder:text-white/45 focus:border-white/35 focus:outline-none sm:w-64"
            aria-label="Search this library"
          />
        </div>
      </div>

      {activeFileFilterCount > 0 && (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-[11px] uppercase tracking-wide text-white/40">
            Active filters
          </span>
          {(["resolutions", "hdr", "codecs"] as const).flatMap((key) => {
            const active =
              key === "resolutions"
                ? initialResolutions
                : key === "hdr"
                  ? initialHdr
                  : initialCodecs;
            return active.map((v) => (
              <button
                key={`${key}:${v}`}
                type="button"
                onClick={() =>
                  toggleMulti(key, v, active)
                }
                className="group inline-flex items-center gap-1 rounded-full border border-accent/30 bg-accent/10 px-2.5 py-0.5 text-[11.5px] text-accent transition-colors hover:border-accent/60"
                aria-label={`Remove filter: ${FILTER_OPTION_LABEL[v] ?? v}`}
              >
                <span>{FILTER_OPTION_LABEL[v] ?? v}</span>
                <span
                  className="text-accent/70 transition-colors group-hover:text-accent"
                  aria-hidden
                >
                  ×
                </span>
              </button>
            ));
          })}
          <button
            type="button"
            onClick={clearFileFilters}
            className="text-[11.5px] text-white/55 underline-offset-2 hover:text-white hover:underline"
          >
            Clear all
          </button>
        </div>
      )}

      {initialFilter === "unmatched" && (
        <div className="rounded-md border border-amber-500/30 bg-amber-500/5 px-4 py-3 text-[12.5px] text-amber-200">
          These titles couldn&apos;t be auto-matched from their filenames.
          Click into one and use <strong>Fix Match</strong> in the modal to
          pick the right TMDB / AniList entry. Doing this once also teaches
          the matcher for similarly-named files.
        </div>
      )}

      {items.length === 0 ? (
        <div className="mx-auto max-w-md py-10 text-center">
          <div className="mx-auto mb-4 flex h-12 w-12 items-center justify-center rounded-full border border-white/10 bg-white/5 text-white/55">
            <svg
              width="22"
              height="22"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.75"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden
            >
              <path d="M3 6h18M6 12h12M10 18h4" />
            </svg>
          </div>
          <h2 className="text-base font-semibold text-white">
            {query ? `No results for "${query}"` : "Nothing matches"}
          </h2>
          <p className="mt-1.5 text-sm text-white/60">
            {query
              ? "Try a different title or broaden your filters."
              : emptyMessage(initialFilter, initialStatus, initialDecade)}
          </p>
          {hasAnyFilter && (
            <button
              type="button"
              onClick={clearAllFilters}
              className="mt-5 inline-block rounded-md border border-white/20 px-3 py-1.5 text-sm text-white/80 transition-colors hover:border-white/40 hover:text-white"
            >
              Clear all filters
            </button>
          )}
        </div>
      ) : (
        <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
          {items.map((it) => (
            <li key={it.ratingKey}>
              <Card item={it} />
            </li>
          ))}
        </ul>
      )}

      <Pagination
        page={initialPage}
        pageSize={pageSize}
        total={initialTotal}
        onPageChange={setPage}
        onPageSizeChange={setPageSize}
        pageSizes={[24, 60, 120]}
        noun="titles"
      />
    </div>
  );
}

function emptyMessage(
  match: MatchFilter,
  status: StatusFilter,
  decade: DecadeFilter,
): string {
  if (match === "unmatched") {
    return "Every title is auto-matched. Nothing here needs review.";
  }
  if (status === "unwatched") return "You've started everything in this slice.";
  if (status === "in_progress") return "Nothing currently in progress.";
  if (status === "watched") return "Nothing finished yet in this slice.";
  if (decade !== "all") return "No titles from this decade in this library.";
  return "Nothing in this library yet. Trigger a scan from Library → Libraries.";
}

function MatchChips({
  current,
  onChange,
}: {
  current: MatchFilter;
  onChange: (next: MatchFilter) => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {(Object.keys(MATCH_LABEL) as MatchFilter[]).map((f) => (
        <button
          key={f}
          type="button"
          aria-pressed={current === f}
          onClick={() => onChange(f)}
          className={`inline-flex items-center gap-1.5 whitespace-nowrap rounded-md border px-2.5 py-1 text-[12px] transition-colors ${
            current === f
              ? "border-accent/30 bg-accent/10 text-accent"
              : "border-white/10 bg-white/4 text-white/70 hover:border-white/20 hover:text-white"
          }`}
        >
          {MATCH_LABEL[f]}
        </button>
      ))}
    </div>
  );
}

/// Popover anchored to the "Filters" trigger button. Uses a controlled
/// <details> element with a document mousedown listener so the popover
/// closes when the user clicks outside it (native <details> does NOT
/// close on outside clicks by itself).
function FiltersPopover({
  activeCount,
  resolutions,
  hdr,
  codecs,
  onToggle,
  onClear,
}: {
  activeCount: number;
  resolutions: ReadonlyArray<string>;
  hdr: ReadonlyArray<string>;
  codecs: ReadonlyArray<string>;
  onToggle: (
    key: "resolutions" | "hdr" | "codecs",
    value: string,
    current: ReadonlyArray<string>,
  ) => void;
  onClear: () => void;
}) {
  const [open, setOpen] = useState(false);
  const detailsRef = useRef<HTMLDetailsElement>(null);

  // Close when a mousedown fires outside the <details> element.
  useEffect(() => {
    if (!open) return;
    function handleMouseDown(e: MouseEvent) {
      if (detailsRef.current && !detailsRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", handleMouseDown);
    return () => document.removeEventListener("mousedown", handleMouseDown);
  }, [open]);

  return (
    <details ref={detailsRef} className="relative" open={open} onToggle={(e) => setOpen((e.currentTarget as HTMLDetailsElement).open)}>
      <summary
        className={`inline-flex cursor-pointer list-none items-center gap-1.5 whitespace-nowrap rounded-md border px-2.5 py-1 text-[12px] transition-colors [&::-webkit-details-marker]:hidden ${
          activeCount > 0
            ? "border-accent/30 bg-accent/10 text-accent"
            : "border-white/15 bg-black/40 text-white/80 hover:border-white/35"
        }`}
        aria-label="File-level filters"
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
          <line x1="4" y1="6" x2="20" y2="6" />
          <line x1="7" y1="12" x2="17" y2="12" />
          <line x1="10" y1="18" x2="14" y2="18" />
        </svg>
        <span>Filters</span>
        {activeCount > 0 && (
          <span className="ml-1 rounded-full bg-accent/20 px-1.5 text-[10.5px] font-semibold leading-[1.4]">
            {activeCount}
          </span>
        )}
      </summary>
      <div className="absolute right-0 top-full z-30 mt-2 w-72 rounded-lg border border-white/10 bg-background/95 p-4 shadow-2xl backdrop-blur-sm">
        <FilterSection
          title="Resolution"
          options={RESOLUTION_OPTIONS}
          active={resolutions}
          onToggle={(v) => onToggle("resolutions", v, resolutions)}
        />
        <FilterSection
          title="HDR"
          options={HDR_OPTIONS}
          active={hdr}
          onToggle={(v) => onToggle("hdr", v, hdr)}
        />
        <FilterSection
          title="Video codec"
          options={CODEC_OPTIONS}
          active={codecs}
          onToggle={(v) => onToggle("codecs", v, codecs)}
        />
        {activeCount > 0 && (
          <button
            type="button"
            onClick={onClear}
            className="mt-3 w-full rounded-md border border-white/10 px-2 py-1 text-[12px] text-white/70 transition-colors hover:border-white/30 hover:text-white"
          >
            Clear all filters
          </button>
        )}
      </div>
    </details>
  );
}

function FilterSection({
  title,
  options,
  active,
  onToggle,
}: {
  title: string;
  options: ReadonlyArray<{ value: string; label: string }>;
  active: ReadonlyArray<string>;
  onToggle: (value: string) => void;
}) {
  return (
    <div className="mb-3 last:mb-0">
      <div className="mb-1.5 text-[10.5px] font-semibold uppercase tracking-wide text-white/45">
        {title}
      </div>
      <div className="space-y-1">
        {options.map((opt) => {
          const checked = active.includes(opt.value);
          return (
            <label
              key={opt.value}
              className="flex cursor-pointer items-center gap-2 rounded px-1.5 py-1 text-[12.5px] text-white/85 transition-colors hover:bg-white/5"
            >
              <input
                type="checkbox"
                checked={checked}
                onChange={() => onToggle(opt.value)}
                className="h-3.5 w-3.5 accent-accent"
              />
              <span>{opt.label}</span>
            </label>
          );
        })}
      </div>
    </div>
  );
}

function StatusChips({
  current,
  onChange,
}: {
  current: StatusFilter;
  onChange: (next: StatusFilter) => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {(Object.keys(STATUS_LABEL) as StatusFilter[]).map((s) => (
        <button
          key={s}
          type="button"
          aria-pressed={current === s}
          onClick={() => onChange(s)}
          className={`inline-flex items-center gap-1.5 whitespace-nowrap rounded-md border px-2.5 py-1 text-[12px] transition-colors ${
            current === s
              ? "border-accent/30 bg-accent/10 text-accent"
              : "border-white/10 bg-white/4 text-white/70 hover:border-white/20 hover:text-white"
          }`}
        >
          {STATUS_LABEL[s]}
        </button>
      ))}
    </div>
  );
}
