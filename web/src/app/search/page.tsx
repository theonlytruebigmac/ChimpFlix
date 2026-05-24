import { Suspense } from "react";
import { Card } from "@/components/Card";
import { ModalRoot } from "@/components/ModalRoot";
import { MoreToExploreChips } from "@/components/MoreToExploreChips";
import { SearchControls } from "@/components/SearchControls";
import { CardSkeleton } from "@/components/Skeleton";
import { items as itemsApi, type ItemKind } from "@/lib/chimpflix-api";
import { adaptItem } from "@/lib/chimpflix-adapt";
import { requireUser } from "@/lib/chimpflix-server";
import { pluralize } from "@/lib/format";

const DEFAULT_PAGE_SIZE = 60;
const ALLOWED_PAGE_SIZES = [24, 60, 120] as const;

type KindFilter = "all" | "movie" | "show";

export default async function SearchPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const sp = await searchParams;
  const q = typeof sp.q === "string" ? sp.q : undefined;
  await requireUser(q ? `/search?q=${encodeURIComponent(q)}` : "/search");
  const query = q?.trim() ?? "";

  const rawKind = typeof sp.kind === "string" ? sp.kind : "all";
  const kind: KindFilter =
    rawKind === "movie" || rawKind === "show" ? rawKind : "all";
  const page = Math.max(1, Number(sp.page) || 1);
  const requestedPageSize = Number(sp.page_size) || DEFAULT_PAGE_SIZE;
  const pageSize = (ALLOWED_PAGE_SIZES as readonly number[]).includes(
    requestedPageSize,
  )
    ? requestedPageSize
    : DEFAULT_PAGE_SIZE;

  return (
    <main className="relative min-h-screen bg-background">
      <div className="px-4 sm:px-8 md:px-12 pb-24 pt-24 sm:pt-28">
        {!query ? (
          <>
            <h1 className="mb-3 text-4xl font-bold tracking-tight">Search</h1>
            <p className="mb-6 text-white/70">
              Type a title in the box above, or jump into a category.
            </p>
            <MoreToExploreChips />
          </>
        ) : (
          <>
            <h1 className="mb-2 text-4xl font-bold tracking-tight">
              Results for &ldquo;{query}&rdquo;
            </h1>
            <MoreToExploreChips />
            <Suspense
              key={`${query}|${kind}|${page}|${pageSize}`}
              fallback={<ResultsSkeleton />}
            >
              <SearchResults
                query={query}
                kind={kind}
                page={page}
                pageSize={pageSize}
              />
            </Suspense>
          </>
        )}
      </div>
      <ModalRoot />
    </main>
  );
}

async function SearchResults({
  query,
  kind,
  page,
  pageSize,
}: {
  query: string;
  kind: KindFilter;
  page: number;
  pageSize: number;
}) {
  const apiKind: ItemKind | undefined =
    kind === "movie" ? "movie" : kind === "show" ? "show" : undefined;
  const res = await itemsApi.list({
    q: query,
    kind: apiKind,
    page,
    page_size: pageSize,
  });
  if (res.items.length === 0) {
    return (
      <div className="mt-6 space-y-4">
        <SearchControls
          query={query}
          kind={kind}
          page={page}
          pageSize={pageSize}
          total={res.total}
        />
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
              <circle cx="11" cy="11" r="7" />
              <line x1="16.5" y1="16.5" x2="21" y2="21" />
            </svg>
          </div>
          <h2 className="text-base font-semibold text-white">
            No results for &ldquo;{query}&rdquo;
          </h2>
          <p className="mt-1.5 text-sm text-white/60">
            {kind === "all"
              ? "Try a different title or check your spelling."
              : `No ${kind === "movie" ? "movies" : "shows"} match — try "All" to broaden the search.`}
          </p>
        </div>
      </div>
    );
  }
  const items = res.items.map(adaptItem);
  return (
    <div className="mt-6 space-y-4">
      <SearchControls
        query={query}
        kind={kind}
        page={page}
        pageSize={pageSize}
        total={res.total}
      />
      {/* Live region: screen readers announce the new count when results
          arrive (or change due to a kind chip flip / page nav). Visually
          rendered as a small caption. `polite` so it doesn't interrupt
          the user's current speech. */}
      <p
        aria-live="polite"
        className="text-sm text-white/60"
      >
        {pluralize(res.total, "result")}
      </p>
      <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
        {items.map((it) => (
          <li key={it.ratingKey}>
            <Card item={it} />
          </li>
        ))}
      </ul>
    </div>
  );
}

function ResultsSkeleton() {
  return (
    <ul className="mt-8 grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
      {Array.from({ length: 12 }).map((_, i) => (
        <li key={i}>
          <CardSkeleton />
        </li>
      ))}
    </ul>
  );
}
