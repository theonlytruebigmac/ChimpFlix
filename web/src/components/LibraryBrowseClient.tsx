"use client";

/// Grid-style browse-all surface for a library. Lives at
/// `/library/[id]/browse` next to the Netflix-style rail view at
/// `/library/[id]`. Solves two complementary problems:
///
///   1. "Where's that movie I downloaded?" — every title in the
///      library, paginated, sortable.
///   2. "Why isn't my file showing up?" — switch the filter chip
///      to "Unmatched" and you see the stubs the scanner created
///      for files whose names didn't fit the auto-matcher's
///      regex. From here the operator can open the modal and
///      run Fix Match.

import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useState, useTransition } from "react";

import { Card } from "@/components/Card";
import { adaptItem } from "@/lib/chimpflix-adapt";
import type { ItemKind, ItemSort, ListedItem } from "@/lib/chimpflix-api";

type Filter = "all" | "matched" | "unmatched";

const SORT_LABEL: Record<ItemSort, string> = {
  recently_added: "Recently added",
  title: "A → Z",
  year_desc: "Year (newest)",
  year_asc: "Year (oldest)",
  rating_desc: "Rating",
};

const FILTER_LABEL: Record<Filter, string> = {
  all: "All",
  matched: "Auto-matched",
  unmatched: "Unmatched (needs review)",
};

interface Props {
  libraryId: number;
  initialItems: ListedItem[];
  initialTotal: number;
  initialPage: number;
  initialSort: ItemSort;
  initialQuery: string;
  initialFilter: Filter;
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

  // Debounce URL updates as the user types in the search box.
  useEffect(() => {
    const trimmed = query.trim();
    if (trimmed === initialQuery) return; // no change from server-rendered state
    const t = window.setTimeout(() => {
      const params = new URLSearchParams(searchParams?.toString() ?? "");
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
    });
  const setFilter = (next: Filter) =>
    updateParam((p) => {
      if (next === "all") p.delete("filter");
      else p.set("filter", next);
      p.delete("page");
    });
  const setPage = (next: number) =>
    updateParam((p) => {
      if (next <= 1) p.delete("page");
      else p.set("page", String(next));
    });

  const totalPages = Math.max(1, Math.ceil(initialTotal / pageSize));
  const items = initialItems.map(adaptItem);

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center gap-2">
        <FilterChips current={initialFilter} onChange={setFilter} />
        <div className="ml-auto flex flex-wrap items-center gap-2">
          <label className="flex items-center gap-2 text-[12px] text-white/65">
            <span>Sort</span>
            <select
              value={initialSort}
              onChange={(e) => setSort(e.target.value as ItemSort)}
              className="rounded-md border border-white/15 bg-black/40 px-2 py-1 text-[12.5px] text-white/90 focus:border-white/35 focus:outline-none"
            >
              {(
                Object.keys(SORT_LABEL) as ItemSort[]
              ).map((s) => (
                <option key={s} value={s}>
                  {SORT_LABEL[s]}
                </option>
              ))}
            </select>
          </label>
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

      {initialFilter === "unmatched" && (
        <div className="rounded-md border border-amber-500/30 bg-amber-500/5 px-4 py-3 text-[12.5px] text-amber-200">
          These titles couldn&apos;t be auto-matched from their filenames.
          Click into one and use <strong>Fix Match</strong> in the modal to
          pick the right TMDB / AniList entry. Doing this once also teaches
          the matcher for similarly-named files.
        </div>
      )}

      {items.length === 0 ? (
        <div className="rounded-lg border border-dashed border-white/15 bg-white/2 px-6 py-16 text-center text-sm text-white/55">
          {query
            ? `No results for "${query}".`
            : initialFilter === "unmatched"
              ? "Every title is auto-matched. Nothing here needs review."
              : "Nothing in this library yet. Trigger a scan from Library → Libraries."}
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

      {totalPages > 1 && (
        <Pagination
          current={initialPage}
          total={totalPages}
          onSelect={setPage}
        />
      )}
    </div>
  );
}

function FilterChips({
  current,
  onChange,
}: {
  current: Filter;
  onChange: (next: Filter) => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {(Object.keys(FILTER_LABEL) as Filter[]).map((f) => (
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
          {FILTER_LABEL[f]}
        </button>
      ))}
    </div>
  );
}

function Pagination({
  current,
  total,
  onSelect,
}: {
  current: number;
  total: number;
  onSelect: (page: number) => void;
}) {
  // Compact "1 … 4 5 6 … 20" style. Show the first, last, current,
  // and the two on either side; ellipses fill the rest. For small
  // ranges we just render every page.
  function visible(): Array<number | "…"> {
    if (total <= 7) return Array.from({ length: total }, (_, i) => i + 1);
    const out: Array<number | "…"> = [1];
    const start = Math.max(2, current - 2);
    const end = Math.min(total - 1, current + 2);
    if (start > 2) out.push("…");
    for (let i = start; i <= end; i++) out.push(i);
    if (end < total - 1) out.push("…");
    out.push(total);
    return out;
  }
  return (
    <nav
      aria-label="Pagination"
      className="flex flex-wrap items-center justify-center gap-1 pt-4"
    >
      <button
        type="button"
        disabled={current <= 1}
        onClick={() => onSelect(current - 1)}
        className="rounded border border-white/15 px-2.5 py-1 text-xs text-white/80 transition-colors hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-40"
      >
        ‹ Prev
      </button>
      {visible().map((p, i) =>
        p === "…" ? (
          <span
            key={`gap-${i}`}
            aria-hidden
            className="px-1.5 text-xs text-white/40"
          >
            …
          </span>
        ) : (
          <button
            key={p}
            type="button"
            onClick={() => onSelect(p)}
            aria-current={p === current ? "page" : undefined}
            className={`min-w-7 rounded border px-2 py-1 text-xs transition-colors ${
              p === current
                ? "border-accent/40 bg-accent/10 text-accent"
                : "border-white/15 text-white/80 hover:bg-white/5"
            }`}
          >
            {p}
          </button>
        ),
      )}
      <button
        type="button"
        disabled={current >= total}
        onClick={() => onSelect(current + 1)}
        className="rounded border border-white/15 px-2.5 py-1 text-xs text-white/80 transition-colors hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-40"
      >
        Next ›
      </button>
    </nav>
  );
}

