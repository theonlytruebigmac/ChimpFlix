"use client";

/// Client-side pagination footer for `/history`. The server page reads
/// `?page=` / `?page_size=` and fetches the corresponding slice; this
/// component just renders the grid + paginator and pushes URL updates
/// when the user changes pages.
///
/// Matches the LibraryBrowseClient / GenreBrowseClient shape so the
/// three discovery surfaces feel identical.

import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useTransition } from "react";

import { Card } from "@/components/Card";
import { Pagination } from "@/components/ui/Pagination";
import { adaptItem } from "@/lib/chimpflix-adapt";
import type { ListedItem } from "@/lib/chimpflix-api";

interface Props {
  initialItems: ListedItem[];
  initialTotal: number;
  initialPage: number;
  pageSize: number;
}

export function HistoryBrowseClient({
  initialItems,
  initialTotal,
  initialPage,
  pageSize,
}: Props) {
  const router = useRouter();
  const searchParams = useSearchParams();
  const [, startTransition] = useTransition();

  const updateParam = useCallback(
    (mutate: (p: URLSearchParams) => void) => {
      const params = new URLSearchParams(searchParams?.toString() ?? "");
      mutate(params);
      const qs = params.toString();
      const url = qs ? `/history?${qs}` : "/history";
      startTransition(() => router.replace(url, { scroll: false }));
    },
    [router, searchParams],
  );

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
      {items.length === 0 ? (
        <p className="text-white/60">
          Nothing watched yet. Play something and it&apos;ll show up here.
        </p>
      ) : (
        <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
          {items.map((it) => (
            <li key={it.ratingKey}>
              <Card item={it} />
            </li>
          ))}
        </ul>
      )}

      {initialTotal > 0 && (
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
