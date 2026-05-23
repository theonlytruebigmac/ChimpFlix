"use client";

/// Client-side sort + pagination for `/person/[id]`. Mirrors
/// [[CollectionBrowseClient]] — most filmographies fit on one page
/// so we slice / sort entirely client-side, no API pagination
/// contract to maintain. The "Filmography order" default is newest-
/// first since that's what the backend returns and what people
/// generally want when checking an actor.

import { useMemo, useState } from "react";

import { Card } from "@/components/Card";
import { Pagination } from "@/components/ui/Pagination";
import { adaptItem } from "@/lib/chimpflix-adapt";
import type { ListedItem } from "@/lib/chimpflix-api";

type PersonSort =
  | "filmography"
  | "title"
  | "year_asc"
  | "year_desc"
  | "rating_desc";

const SORT_LABEL: Record<PersonSort, string> = {
  filmography: "Filmography (newest)",
  title: "A → Z",
  year_asc: "Year (oldest)",
  year_desc: "Year (newest)",
  rating_desc: "Rating",
};

const DEFAULT_PAGE_SIZE = 60;

export function PersonBrowseClient({
  items: rawItems,
}: {
  items: ListedItem[];
}) {
  const [sort, setSort] = useState<PersonSort>("filmography");
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);

  const sorted = useMemo(() => {
    // "filmography" is the API's default order (year DESC), so we
    // leave the array as-is. Other sorts get a fresh sorted copy.
    if (sort === "filmography") return rawItems;
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
  function handleSortChange(next: PersonSort) {
    setSort(next);
    setPage(1);
  }

  const showControls = rawItems.length > 6;

  if (rawItems.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-white/15 bg-white/2 px-6 py-16 text-center text-sm text-white/55">
        No titles featuring this person in your library.
      </div>
    );
  }

  return (
    <div className="space-y-5">
      {showControls && (
        <div className="flex flex-wrap items-center justify-end gap-2">
          <label className="flex items-center gap-2 text-[12px] text-white/65">
            <span>Sort</span>
            <select
              value={sort}
              onChange={(e) => handleSortChange(e.target.value as PersonSort)}
              className="rounded-md border border-white/15 bg-black/40 px-2 py-1 text-[12.5px] text-white/90 focus:border-white/35 focus:outline-none"
            >
              {(Object.keys(SORT_LABEL) as PersonSort[]).map((s) => (
                <option key={s} value={s}>
                  {SORT_LABEL[s]}
                </option>
              ))}
            </select>
          </label>
        </div>
      )}

      <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
        {adapted.map((it) => (
          <li key={it.ratingKey}>
            <Card item={it} />
          </li>
        ))}
      </ul>

      {total > DEFAULT_PAGE_SIZE && (
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
