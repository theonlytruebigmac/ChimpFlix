"use client";

/// Client-side controls + pagination for `/genre/[name]`. The server
/// page reads URL state (sort / kind / page / page_size) and fetches
/// the data; this component renders the interactive chrome and
/// pushes URL updates that re-trigger the server render.
///
/// Mirrors the LibraryBrowseClient shape so the two grids feel
/// identical. Kept as a separate component (rather than abstracting
/// a shared "BrowseGridClient") because the control sets differ:
/// Library has the auto-match filter chips, Genre has the
/// movie/show kind toggle. Premature abstraction would muddle both.

import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useRef, useTransition } from "react";

import { Card } from "@/components/Card";
import { Pagination } from "@/components/ui/Pagination";
import { adaptItem } from "@/lib/chimpflix-adapt";
import type { ItemSort, ListedItem } from "@/lib/chimpflix-api";

export type GenreKindFilter = "all" | "movie" | "show";

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

interface Props {
  /// URL-segment-encoded name (so we can rebuild the route without
  /// re-encoding). The display title is rendered by the parent page.
  genreSegment: string;
  initialItems: ListedItem[];
  initialTotal: number;
  initialPage: number;
  initialSort: ItemSort;
  initialKind: GenreKindFilter;
  /** Echoes the seed the route page resolved for the current page.
   *  Null when sort !== "random". */
  initialSeed: number | null;
  pageSize: number;
}

export function GenreBrowseClient({
  genreSegment,
  initialItems,
  initialTotal,
  initialPage,
  initialSort,
  initialKind,
  initialSeed,
  pageSize,
}: Props) {
  const router = useRouter();
  const searchParams = useSearchParams();
  const [, startTransition] = useTransition();

  // Mirror searchParams into a ref so any future debounced timers
  // would read the LATEST snapshot, matching the LibraryBrowseClient
  // pattern. Currently only the inline handlers below use it.
  const searchParamsRef = useRef(searchParams);
  useEffect(() => {
    searchParamsRef.current = searchParams;
  }, [searchParams]);

  const updateParam = useCallback(
    (mutate: (p: URLSearchParams) => void) => {
      const params = new URLSearchParams(searchParams?.toString() ?? "");
      mutate(params);
      const qs = params.toString();
      const url = qs
        ? `/genre/${genreSegment}?${qs}`
        : `/genre/${genreSegment}`;
      startTransition(() => router.replace(url, { scroll: false }));
    },
    [router, searchParams, genreSegment],
  );

  const setSort = (next: ItemSort) =>
    updateParam((p) => {
      p.set("sort", next);
      p.delete("page");
      // Random sort needs a stable seed so pagination doesn't reshuffle.
      // Generate one when the user picks Shuffle; strip it on any other
      // sort so stale ?seed= doesn't linger in the URL.
      if (next === "random") {
        if (!p.get("seed")) {
          p.set("seed", String(Math.floor(Math.random() * 1_000_000_000)));
        }
      } else {
        p.delete("seed");
      }
    });
  const reshuffle = () =>
    updateParam((p) => {
      p.set("sort", "random");
      p.set("seed", String(Math.floor(Math.random() * 1_000_000_000)));
      p.delete("page");
    });
  const setKind = (next: GenreKindFilter) =>
    updateParam((p) => {
      if (next === "all") p.delete("type");
      else p.set("type", next);
      p.delete("page");
    });
  const setPage = (next: number) => {
    updateParam((p) => {
      if (next <= 1) p.delete("page");
      else p.set("page", String(next));
    });
    if (typeof window !== "undefined") {
      window.scrollTo({ top: 0, behavior: "smooth" });
    }
  };
  const setPageSize = (next: number) =>
    updateParam((p) => {
      if (next === 60) p.delete("page_size");
      else p.set("page_size", String(next));
      p.delete("page");
    });

  const items = initialItems.map(adaptItem);

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex flex-wrap items-center gap-1.5">
          {KIND_OPTIONS.map(({ value, label }) => (
            <button
              key={value}
              type="button"
              aria-pressed={initialKind === value}
              onClick={() => setKind(value)}
              className={`inline-flex items-center gap-1.5 whitespace-nowrap rounded-md border px-2.5 py-1 text-[12px] transition-colors ${
                initialKind === value
                  ? "border-accent/30 bg-accent/10 text-accent"
                  : "border-white/10 bg-white/4 text-white/70 hover:border-white/20 hover:text-white"
              }`}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="ml-auto flex flex-wrap items-center gap-2">
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
        </div>
      </div>

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
            Nothing matches
          </h2>
          <p className="mt-1.5 text-sm text-white/60">
            {initialKind === "all"
              ? "Nothing in this genre yet."
              : initialKind === "movie"
                ? "No movies in this genre yet."
                : "No shows in this genre yet."}
          </p>
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

      {initialTotal > pageSize && (
        <Pagination
          page={initialPage}
          pageSize={pageSize}
          total={initialTotal}
          onPageChange={setPage}
          onPageSizeChange={setPageSize}
          pageSizes={[24, 60, 120]}
          noun="titles"
        />
      )}
    </div>
  );
}

const KIND_OPTIONS: ReadonlyArray<{ value: GenreKindFilter; label: string }> = [
  { value: "all", label: "All" },
  { value: "movie", label: "Movies" },
  { value: "show", label: "Shows" },
];
