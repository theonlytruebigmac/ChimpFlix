import { Suspense } from "react";
import { ModalRoot } from "@/components/ModalRoot";
import { Rail } from "@/components/Rail";
import { RailSkeleton } from "@/components/Skeleton";
import { Top10Rail } from "@/components/Top10Rail";
import {
  items as itemsApi,
  libraries as librariesApi,
  prefs as prefsApi,
  type ListedItem,
} from "@/lib/chimpflix-api";
import { adaptItem } from "@/lib/chimpflix-adapt";
import { requireUser } from "@/lib/chimpflix-server";

const RAIL_SIZE = 20;

/// Discovery surface for "what's hot right now." Home already covers
/// the long tail (per-library "New in X" rails + Recently Added), so
/// this page intentionally stays narrow:
///   • Top 10 trending movies / shows (TMDB popularity, refreshed
///     weekly — the only globally-curated signal we have)
///   • Top Rated movies / shows (audience rating, sourced locally —
///     useful "what should I watch from my library" angle Home
///     doesn't surface)
/// The per-library "New in X" rails that used to live here were
/// strict duplicates of Home and have been removed to keep this
/// page differentiated.
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
  // Allow-list reused by every rail to honor hidden / user-hidden
  // libraries.
  const visibleLibIds = libraries.map((l) => l.id);

  return (
    <main className="relative min-h-screen bg-background">
      <div className="pb-24 pt-24 sm:pt-28">
        <h1 className="mb-8 px-4 sm:px-8 md:px-12 text-4xl font-bold tracking-tight">
          New &amp; Popular
        </h1>
        <div className="space-y-1">
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
        </div>
      </div>
      <ModalRoot />
    </main>
  );
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
  let raw: Array<ListedItem & { rank: number }>;
  try {
    const res = await itemsApi.trending(kind, 10, visibleLibIds);
    raw = res.items;
  } catch {
    return null;
  }
  // Dedupe before adapting so we can read the raw tmdb_id field.
  // The same title can legitimately exist in multiple libraries
  // (e.g. Spirited Away in both Anime and Movies) — the cards
  // would otherwise render twice.
  //
  // Re-rank to 1..N after dedupe + library-intersection. The Trakt /
  // TMDB ranks the upstream sends are global, so when items outside
  // our library are filtered out we'd otherwise render gaps (3, 4, 5,
  // 7, …). Netflix shows a clean 1, 2, 3 sequence — the relative
  // order is what matters, not the absolute popularity number.
  const entries = dedupeByTmdb(raw).map(({ rank: _rank, ...item }, idx) => ({
    rank: idx + 1,
    item: adaptItem(item),
  }));
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
  // Over-fetch so we still fill the rail after collapsing cross-library
  // duplicates.
  const res = await itemsApi.list({
    kind,
    sort: "rating_desc",
    page_size: RAIL_SIZE * 2,
    library_ids: visibleLibIds,
  });
  const items = dedupeByTmdb(res.items)
    .slice(0, RAIL_SIZE)
    .map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title={title} items={items} />;
}

/// Collapse rows with the same TMDB id, keeping the first occurrence
/// (which the caller's ORDER BY already ranked highest). Items
/// without a tmdb_id key by row id (no collapse). Used by discovery
/// rails that pull globally; browse pages keep per-library scoping.
function dedupeByTmdb<T extends { tmdb_id: number | null; id: number }>(
  items: T[],
): T[] {
  const seen = new Set<string>();
  const out: T[] = [];
  for (const it of items) {
    const key = it.tmdb_id != null ? `tmdb:${it.tmdb_id}` : `id:${it.id}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(it);
  }
  return out;
}
