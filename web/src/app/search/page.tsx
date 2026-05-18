import { Suspense } from "react";
import { Card } from "@/components/Card";
import { ModalRoot } from "@/components/ModalRoot";
import { MoreToExploreChips } from "@/components/MoreToExploreChips";
import { CardSkeleton } from "@/components/Skeleton";
import { TopNav } from "@/components/TopNav";
import { items as itemsApi } from "@/lib/chimpflix-api";
import { adaptItem } from "@/lib/chimpflix-adapt";
import { requireUser } from "@/lib/chimpflix-server";

const RESULTS_PAGE_SIZE = 60;

export default async function SearchPage({
  searchParams,
}: {
  searchParams: Promise<{ q?: string }>;
}) {
  const { q } = await searchParams;
  await requireUser(q ? `/search?q=${encodeURIComponent(q)}` : "/search");
  const query = q?.trim() ?? "";

  return (
    <main className="relative min-h-screen bg-background">
      <TopNav />
      <div className="px-12 pb-24 pt-28">
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
            <Suspense fallback={<ResultsSkeleton />}>
              <SearchResults query={query} />
            </Suspense>
          </>
        )}
      </div>
      <ModalRoot />
    </main>
  );
}

async function SearchResults({ query }: { query: string }) {
  const res = await itemsApi.list({ q: query, page_size: RESULTS_PAGE_SIZE });
  if (res.items.length === 0) {
    return (
      <p className="mt-8 text-white/70">
        No results. Try a different title or check your spelling.
      </p>
    );
  }
  const items = res.items.map(adaptItem);
  return (
    <>
      <p className="mb-6 text-sm text-white/60">
        {res.total} {res.total === 1 ? "result" : "results"}
      </p>
      <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
        {items.map((it) => (
          <li key={it.ratingKey}>
            <Card item={it} />
          </li>
        ))}
      </ul>
    </>
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
