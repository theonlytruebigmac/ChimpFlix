import { Suspense } from "react";
import { ModalRoot } from "@/components/ModalRoot";
import { Rail } from "@/components/Rail";
import { RailSkeleton } from "@/components/Skeleton";
import { Top10Rail } from "@/components/Top10Rail";
import { brandName } from "@/lib/env";
import {
  items as itemsApi,
  libraries as librariesApi,
  prefs as prefsApi,
  type Library,
} from "@/lib/chimpflix-api";
import { adaptItem } from "@/lib/chimpflix-adapt";
import { requireUser } from "@/lib/chimpflix-server";

const RAIL_SIZE = 20;

export default async function NewPopularPage() {
  await requireUser("/new-popular");
  const [{ libraries: allLibs }, { library_ids: hiddenIds }] = await Promise.all([
    librariesApi.list(),
    prefsApi.hiddenLibraries(),
  ]);
  const hidden = new Set(hiddenIds);
  const libraries = allLibs.filter(
    (l) => l.visibility !== "hidden" && !hidden.has(l.id),
  );
  // Single allow-list reused by every rail on this page. Per-library rails
  // already scope to one library so the filter is redundant there; the
  // global rails ("New on …", "Top Rated …", trending) use it to stop
  // hidden / user-hidden libraries from leaking into Browse.
  const visibleLibIds = libraries.map((l) => l.id);

  return (
    <main className="relative min-h-screen bg-background">
      <div className="pb-24 pt-24 sm:pt-28">
        <h1 className="mb-8 px-4 sm:px-8 md:px-12 text-4xl font-bold tracking-tight">
          New &amp; Popular
        </h1>
        <div className="space-y-1">
          <Suspense fallback={<RailSkeleton title={`New on ${brandName()}`} />}>
            <NewOnBrandRail visibleLibIds={visibleLibIds} />
          </Suspense>
          <Suspense fallback={null}>
            <Top10TrendingRail
              kind="movie"
              title="Top 10 Movies This Week"
              visibleLibIds={visibleLibIds}
            />
          </Suspense>
          <Suspense fallback={null}>
            <Top10TrendingRail
              kind="show"
              title="Top 10 Shows This Week"
              visibleLibIds={visibleLibIds}
            />
          </Suspense>
          <Suspense fallback={<RailSkeleton title="Top Rated Movies" />}>
            <TopRatedRail
              kind="movie"
              title="Top Rated Movies"
              visibleLibIds={visibleLibIds}
            />
          </Suspense>
          <Suspense fallback={<RailSkeleton title="Top Rated Shows" />}>
            <TopRatedRail
              kind="show"
              title="Top Rated Shows"
              visibleLibIds={visibleLibIds}
            />
          </Suspense>
          {libraries.map((lib) => (
            <Suspense
              key={`lib-${lib.id}`}
              fallback={<RailSkeleton title={`New in ${lib.name}`} />}
            >
              <LibraryRail lib={lib} />
            </Suspense>
          ))}
        </div>
      </div>
      <ModalRoot />
    </main>
  );
}

async function NewOnBrandRail({
  visibleLibIds,
}: {
  visibleLibIds: number[];
}) {
  if (visibleLibIds.length === 0) return null;
  const res = await itemsApi.list({
    page_size: RAIL_SIZE,
    library_ids: visibleLibIds,
  });
  const items = res.items.map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title={`New on ${brandName()}`} items={items} />;
}

async function Top10TrendingRail({
  kind,
  title,
  visibleLibIds,
}: {
  kind: "movie" | "show";
  title: string;
  visibleLibIds: number[];
}) {
  // No-op when TMDB isn't configured or refresh_trending hasn't run.
  if (visibleLibIds.length === 0) return null;
  let entries: Array<{ rank: number; item: ReturnType<typeof adaptItem> }>;
  try {
    const res = await itemsApi.trending(kind, 10, visibleLibIds);
    entries = res.items.map(({ rank, ...item }) => ({
      rank,
      item: adaptItem(item),
    }));
  } catch {
    return null;
  }
  if (entries.length === 0) return null;
  return <Top10Rail title={title} items={entries} />;
}

async function TopRatedRail({
  kind,
  title,
  visibleLibIds,
}: {
  kind: "movie" | "show";
  title: string;
  visibleLibIds: number[];
}) {
  if (visibleLibIds.length === 0) return null;
  const res = await itemsApi.list({
    kind,
    sort: "rating_desc",
    page_size: RAIL_SIZE,
    library_ids: visibleLibIds,
  });
  const items = res.items.map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title={title} items={items} />;
}

async function LibraryRail({ lib }: { lib: Library }) {
  const res = await itemsApi.list({
    library_id: lib.id,
    page_size: RAIL_SIZE,
  });
  const items = res.items.map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title={`New in ${lib.name}`} items={items} />;
}
