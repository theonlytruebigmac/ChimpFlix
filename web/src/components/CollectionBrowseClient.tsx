"use client";

/// Client-side sort + pagination for `/collection/[id]`. The
/// collection-detail endpoint returns every item inline (collections
/// are typically small — 5-50 items for franchises, occasionally
/// larger for manual curations); we slice / sort entirely client-
/// side so the existing API doesn't need a new pagination contract.
///
/// Default sort is "Collection order" — i.e. whatever the API
/// returned. For TMDB-imported franchises that's typically release
/// order; for manual collections it's the operator's curation. We
/// never overwrite that without an explicit user pick.

import Link from "next/link";
import { useMemo, useState } from "react";

import { Card } from "@/components/Card";
import { Pagination } from "@/components/ui/Pagination";
import { adaptItem } from "@/lib/chimpflix-adapt";
import type { ListedItem } from "@/lib/chimpflix-api";

type CollectionSort =
  | "collection_order"
  | "title"
  | "year_asc"
  | "year_desc"
  | "rating_desc";

const SORT_LABEL: Record<CollectionSort, string> = {
  collection_order: "Collection order",
  title: "A → Z",
  year_asc: "Year (oldest)",
  year_desc: "Year (newest)",
  rating_desc: "Rating",
};

const DEFAULT_PAGE_SIZE = 60;

export function CollectionBrowseClient({
  items: rawItems,
}: {
  items: ListedItem[];
}) {
  const [sort, setSort] = useState<CollectionSort>("collection_order");
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);

  const sorted = useMemo(() => {
    if (sort === "collection_order") return rawItems;
    // Cheap stable sort by the picked axis. Items without a year /
    // rating sink to the end so picking "year asc" doesn't surface
    // unknowns ahead of real release dates.
    const arr = [...rawItems];
    switch (sort) {
      case "title":
        arr.sort((a, b) => a.title.localeCompare(b.title));
        break;
      case "year_asc":
        arr.sort(
          (a, b) =>
            (a.year ?? Number.MAX_SAFE_INTEGER) -
            (b.year ?? Number.MAX_SAFE_INTEGER),
        );
        break;
      case "year_desc":
        arr.sort((a, b) => (b.year ?? -1) - (a.year ?? -1));
        break;
      case "rating_desc":
        arr.sort(
          (a, b) => (b.rating_audience ?? -1) - (a.rating_audience ?? -1),
        );
        break;
    }
    return arr;
  }, [rawItems, sort]);

  const total = sorted.length;
  const start = (page - 1) * pageSize;
  const visible = sorted.slice(start, start + pageSize);
  const adapted = visible.map(adaptItem);

  function handlePageChange(next: number) {
    setPage(next);
    if (typeof window !== "undefined") {
      window.scrollTo({ top: 0, behavior: "smooth" });
    }
  }
  function handlePageSizeChange(next: number) {
    setPageSize(next);
    setPage(1);
  }
  function handleSortChange(next: CollectionSort) {
    setSort(next);
    setPage(1);
  }

  // Hide the controls + paginator entirely when the collection is
  // small enough that sort/pagination would just be visual noise.
  const showControls = rawItems.length > 6;

  return (
    <div className="space-y-5">
      {showControls && (
        <div className="flex flex-wrap items-center justify-end gap-2">
          <label className="flex items-center gap-2 text-[12px] text-white/65">
            <span>Sort</span>
            <select
              value={sort}
              onChange={(e) => handleSortChange(e.target.value as CollectionSort)}
              className="rounded-md border border-white/15 bg-black/40 px-2 py-1 text-[12.5px] text-white/90 focus:border-white/35 focus:outline-none"
            >
              {(Object.keys(SORT_LABEL) as CollectionSort[]).map((s) => (
                <option key={s} value={s}>
                  {SORT_LABEL[s]}
                </option>
              ))}
            </select>
          </label>
        </div>
      )}

      {adapted.length === 0 ? (
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
              <rect x="3" y="3" width="7" height="7" rx="1.5" />
              <rect x="14" y="3" width="7" height="7" rx="1.5" />
              <rect x="3" y="14" width="7" height="7" rx="1.5" />
              <rect x="14" y="14" width="7" height="7" rx="1.5" />
            </svg>
          </div>
          <h2 className="text-base font-semibold text-white">
            No titles in this collection yet
          </h2>
          <p className="mt-1.5 text-sm text-white/60">
            Items will show up here once they&apos;re added to the library.
          </p>
          <Link
            href="/"
            className="mt-5 inline-block text-sm text-white underline underline-offset-4 hover:text-(--color-accent)"
          >
            Browse titles
          </Link>
        </div>
      ) : (
        <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
          {adapted.map((it) => (
            <li key={it.ratingKey}>
              <Card item={it} />
            </li>
          ))}
        </ul>
      )}

      {total > pageSize && (
        <Pagination
          page={page}
          pageSize={pageSize}
          total={total}
          onPageChange={handlePageChange}
          onPageSizeChange={handlePageSizeChange}
          pageSizes={[24, 60, 120]}
          noun="titles"
        />
      )}
    </div>
  );
}
