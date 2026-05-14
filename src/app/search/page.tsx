import { Suspense } from "react";
import { Card } from "@/components/Card";
import { ModalRoot } from "@/components/ModalRoot";
import { CardSkeleton } from "@/components/Skeleton";
import { TopNav } from "@/components/TopNav";
import {
  filterHiddenItems,
  readHiddenLibraries,
} from "@/lib/library-prefs";
import { searchHubs, type ServerAuth } from "@/lib/plex-data";
import { requireServerAuth } from "@/lib/session";

export default async function SearchPage({
  searchParams,
}: {
  searchParams: Promise<{ q?: string }>;
}) {
  const { q } = await searchParams;
  const auth = await requireServerAuth();
  const query = q?.trim() ?? "";

  return (
    <main className="relative min-h-screen bg-black">
      <TopNav />
      <div className="px-12 pb-24 pt-28">
        {!query ? (
          <>
            <h1 className="mb-3 text-4xl font-bold tracking-tight">Search</h1>
            <p className="text-white/70">
              Type a title, episode, or genre in the box above.
            </p>
          </>
        ) : (
          <>
            <h1 className="mb-2 text-4xl font-bold tracking-tight">
              Results for &ldquo;{query}&rdquo;
            </h1>
            <Suspense fallback={<ResultsSkeleton />}>
              <SearchResults auth={auth} query={query} />
            </Suspense>
          </>
        )}
      </div>
      <ModalRoot />
    </main>
  );
}

async function SearchResults({
  auth,
  query,
}: {
  auth: ServerAuth;
  query: string;
}) {
  const [rawHubs, hidden] = await Promise.all([
    searchHubs(auth, query),
    readHiddenLibraries(),
  ]);
  const hubs = rawHubs
    .map((hub) => ({ ...hub, items: filterHiddenItems(hub.items, hidden) }))
    .filter((hub) => hub.items.length > 0);

  if (hubs.length === 0) {
    return (
      <p className="mt-8 text-white/60">No matches in your Plex libraries.</p>
    );
  }
  return (
    <div className="mt-10 space-y-14">
      {hubs.map((hub) => (
        <section key={hub.type} className="zf-rise-in">
          <h2 className="mb-5 text-xl font-semibold tracking-tight">
            {hub.title}
          </h2>
          <ul className="flex flex-wrap gap-3">
            {hub.items.map((item) => (
              <li key={item.ratingKey} className="flex-none">
                <Card item={item} />
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

function ResultsSkeleton() {
  return (
    <div className="mt-10 space-y-14">
      {Array.from({ length: 2 }).map((_, secIdx) => (
        <section key={secIdx}>
          <div className="mb-5 h-7 w-32 rounded bg-white/10" />
          <ul className="flex flex-wrap gap-3">
            {Array.from({ length: 6 }).map((_, i) => (
              <li key={i} className="flex-none">
                <CardSkeleton />
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}
