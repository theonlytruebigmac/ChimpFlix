"use client";

/// Client-side chrome for `/search`: kind filter chips + pagination.
/// The /search page itself is server-rendered (auth + initial fetch),
/// but kind/page navigation needs to round-trip the URL through the
/// router. This component handles both, using router.replace so the
/// browser-history stack doesn't bloat when a user flicks between
/// chips or pages within a single search session.

import { useRouter, useSearchParams } from "next/navigation";
import { useTransition } from "react";

import { Pagination } from "@/components/ui/Pagination";

type KindFilter = "all" | "movie" | "show";

const KIND_OPTIONS: ReadonlyArray<{ value: KindFilter; label: string }> = [
  { value: "all", label: "All" },
  { value: "movie", label: "Movies" },
  { value: "show", label: "Shows" },
];

interface Props {
  /// Current values resolved by the server route, so this client
  /// component stays in lockstep with the SSR result. Changing any
  /// of them issues a `router.replace` that re-runs the server
  /// component with new params.
  query: string;
  kind: KindFilter;
  page: number;
  pageSize: number;
  total: number;
}

export function SearchControls({
  query,
  kind,
  page,
  pageSize,
  total,
}: Props) {
  const router = useRouter();
  const searchParams = useSearchParams();
  const [pending, startTransition] = useTransition();

  function setParam(mutate: (p: URLSearchParams) => void) {
    const next = new URLSearchParams(searchParams?.toString() ?? "");
    mutate(next);
    startTransition(() => {
      router.replace(`/search?${next.toString()}`, { scroll: false });
    });
  }

  function setKind(k: KindFilter) {
    setParam((p) => {
      if (k === "all") p.delete("kind");
      else p.set("kind", k);
      p.delete("page"); // changing the filter resets paging
    });
  }

  function setPage(next: number) {
    setParam((p) => {
      if (next <= 1) p.delete("page");
      else p.set("page", String(next));
    });
    if (typeof window !== "undefined") {
      window.scrollTo({ top: 0, behavior: "smooth" });
    }
  }

  function setPageSize(next: number) {
    setParam((p) => {
      if (next === 60) p.delete("page_size");
      else p.set("page_size", String(next));
      p.delete("page");
    });
  }

  // Render nothing when there's no query yet — controls below the
  // empty state would just be visual noise.
  if (!query) return null;

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-1.5">
        {KIND_OPTIONS.map((opt) => (
          <button
            key={opt.value}
            type="button"
            aria-pressed={kind === opt.value}
            onClick={() => setKind(opt.value)}
            disabled={pending}
            className={`inline-flex items-center gap-1.5 whitespace-nowrap rounded-md border px-2.5 py-1 text-[12px] transition-colors disabled:opacity-60 ${
              kind === opt.value
                ? "border-accent/30 bg-accent/10 text-accent"
                : "border-white/10 bg-white/4 text-white/70 hover:border-white/20 hover:text-white"
            }`}
          >
            {opt.label}
          </button>
        ))}
      </div>
      {/* Pagination's `Pager*` buttons absorb the noun + per-page
          selector + page list. Suppress the whole thing on a single
          page of results (the component renders a tighter footer
          automatically when totalPages === 1, but on /search the
          common case is "exactly one page" and even the "Show 60
          per page" row reads as chrome the user doesn't need). */}
      {total > pageSize && (
        <Pagination
          page={page}
          pageSize={pageSize}
          total={total}
          onPageChange={setPage}
          onPageSizeChange={setPageSize}
          pageSizes={[24, 60, 120]}
          noun="results"
        />
      )}
    </div>
  );
}
