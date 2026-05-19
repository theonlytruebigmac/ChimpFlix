import { Suspense } from "react";
import { CollectionsRail } from "@/components/CollectionsRail";
import { Hero } from "@/components/Hero";
import { ModalRoot } from "@/components/ModalRoot";
import { Rail } from "@/components/Rail";
import { RailErrorBoundary } from "@/components/RailErrorBoundary";
import { HeroSkeleton, RailSkeleton } from "@/components/Skeleton";
import { Top10Rail } from "@/components/Top10Rail";
import { TopNav } from "@/components/TopNav";
import { pickHeroIndex } from "@/lib/hero";
import {
  collections as collectionsApi,
  items as itemsApi,
  libraries as librariesApi,
  playState as playStateApi,
  prefs as prefsApi,
  type Library,
} from "@/lib/chimpflix-api";
import { adaptItem, adaptOnDeck } from "@/lib/chimpflix-adapt";
import { requireUser } from "@/lib/chimpflix-server";
import type { MediaItem } from "@/lib/chimpflix-types";

const RAIL_PAGE_SIZE = 20;
const MOVIE_GENRES = ["Action", "Comedy", "Drama"];
const SHOW_GENRES = ["Drama", "Comedy", "Animation"];

export default async function Home() {
  await requireUser("/");
  const [{ libraries: allLibs }, { library_ids: hiddenIds }] =
    await Promise.all([librariesApi.list(), prefsApi.hiddenLibraries()]);
  const hidden = new Set(hiddenIds);
  const libs = allLibs.filter(
    (l) => l.visibility !== "hidden" && !hidden.has(l.id),
  );
  const firstMovieLib = libs.find((l) => l.kind === "movies");
  const firstShowLib = libs.find((l) => l.kind === "shows");
  // Global rails (Hero / Recently Added) intersect against this set so
  // hidden / user-hidden libraries don't leak in.
  const visibleLibIds = libs.map((l) => l.id);

  return (
    <main className="relative">
      <TopNav />
      <RailErrorBoundary label="HomeHero">
        <Suspense fallback={<HeroSkeleton />}>
          <HomeHero visibleLibIds={visibleLibIds} />
        </Suspense>
      </RailErrorBoundary>
      <div className="relative z-20 space-y-1 pb-24 pt-4">
        <RailErrorBoundary label="ContinueWatching">
          <Suspense fallback={<RailSkeleton title="Continue Watching" />}>
            <ContinueWatchingRail />
          </Suspense>
        </RailErrorBoundary>
        <RailErrorBoundary label="RecentlyAdded">
          <Suspense fallback={<RailSkeleton title="Recently Added" />}>
            <RecentlyAddedRail visibleLibIds={visibleLibIds} />
          </Suspense>
        </RailErrorBoundary>
        <RailErrorBoundary label="Top10Movies">
          <Suspense fallback={null}>
            <Top10TrendingRail
              kind="movie"
              title="Top 10 Movies This Week"
              visibleLibIds={visibleLibIds}
            />
          </Suspense>
        </RailErrorBoundary>
        <RailErrorBoundary label="Top10Shows">
          <Suspense fallback={null}>
            <Top10TrendingRail
              kind="show"
              title="Top 10 Shows This Week"
              visibleLibIds={visibleLibIds}
            />
          </Suspense>
        </RailErrorBoundary>
        <RailErrorBoundary label="Collections">
          <Suspense fallback={<RailSkeleton title="Collections" />}>
            <HomeCollectionsRail />
          </Suspense>
        </RailErrorBoundary>
        {libs.map((lib) => (
          <RailErrorBoundary key={`lib-${lib.id}`} label={`Lib:${lib.name}`}>
            <Suspense fallback={<RailSkeleton title={`New in ${lib.name}`} />}>
              <LibSectionRail lib={lib} />
            </Suspense>
          </RailErrorBoundary>
        ))}
        {firstMovieLib &&
          MOVIE_GENRES.map((g) => (
            <RailErrorBoundary key={`movie-genre-${g}`} label={`MovieGenre:${g}`}>
              <Suspense fallback={null}>
                <GenreRail libraryId={firstMovieLib.id} kind="movie" genre={g} />
              </Suspense>
            </RailErrorBoundary>
          ))}
        {firstShowLib &&
          SHOW_GENRES.map((g) => (
            <RailErrorBoundary key={`show-genre-${g}`} label={`ShowGenre:${g}`}>
              <Suspense fallback={null}>
                <GenreRail libraryId={firstShowLib.id} kind="show" genre={g} />
              </Suspense>
            </RailErrorBoundary>
          ))}
      </div>
      <ModalRoot />
    </main>
  );
}

async function HomeHero({ visibleLibIds }: { visibleLibIds: number[] }) {
  // On-deck is whatever the user is in the middle of — never something
  // they explicitly hid via prefs, so we skip the library filter there.
  // The "recent" fallback pool needs the filter to avoid surfacing fresh
  // imports from a hidden library as a hero card.
  const [deckRes, latest] = await Promise.all([
    playStateApi.onDeck(),
    visibleLibIds.length === 0
      ? Promise.resolve({ items: [] as Awaited<ReturnType<typeof itemsApi.list>>["items"] })
      : itemsApi.list({ page_size: 12, library_ids: visibleLibIds }),
  ]);
  // Accept items with either a true backdrop (`art`) or just a poster
  // (`thumb`) — Hero.tsx already does `art ?? thumb`. Requiring backdrop
  // collapses the hero to null on libraries whose metadata source only
  // shipped posters (common for anime), which in turn collapses the
  // whole layout because the rails container has no nav clearance.
  // Prefer art-bearing candidates by listing them first.
  const hasImage = (it: MediaItem) => Boolean(it.art) || Boolean(it.thumb);
  const onDeck = deckRes.items.map(adaptOnDeck).filter(hasImage);
  const recent = latest.items
    .map(adaptItem)
    .filter((it) => hasImage(it) && (it.type === "movie" || it.type === "show"));
  const pool = [...onDeck, ...recent]
    .sort((a, b) => (a.art ? -1 : 0) - (b.art ? -1 : 0))
    .slice(0, 5);
  if (pool.length === 0) {
    // Genuinely empty library: render a nav-height spacer so the first
    // rail's title doesn't slide under the fixed TopNav. Cheaper than
    // wedging conditional padding into the rails container, which would
    // leave a visible gap under the hero in the common case.
    return <div className="h-20 md:h-24" aria-hidden />;
  }
  return <Hero item={pool[pickHeroIndex(pool, "home")]} />;
}

async function ContinueWatchingRail() {
  const res = await playStateApi.onDeck();
  const items = res.items.map(adaptOnDeck);
  if (items.length === 0) return null;
  return <Rail title="Continue Watching" items={items} />;
}

async function RecentlyAddedRail({
  visibleLibIds,
}: {
  visibleLibIds: number[];
}) {
  if (visibleLibIds.length === 0) return null;
  const res = await itemsApi.list({
    page_size: RAIL_PAGE_SIZE,
    library_ids: visibleLibIds,
  });
  const items = res.items.map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title="Recently Added" items={items} />;
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
  // No-op render when TMDB isn't wired or the refresh task hasn't run.
  // The endpoint returns 200 with an empty array in those cases, so we
  // just bail without surfacing an error to the user.
  if (visibleLibIds.length === 0) return null;
  let entries: Array<{ rank: number; item: ReturnType<typeof adaptItem> }>;
  try {
    const res = await itemsApi.trending(kind, 10, visibleLibIds);
    entries = res.items
      .map(({ rank, ...item }) => ({ rank, item: adaptItem(item) }));
  } catch {
    return null;
  }
  if (entries.length === 0) return null;
  return <Top10Rail title={title} items={entries} />;
}

async function LibSectionRail({ lib }: { lib: Library }) {
  const res = await itemsApi.list({
    library_id: lib.id,
    page_size: RAIL_PAGE_SIZE,
  });
  const items = res.items.map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title={`New in ${lib.name}`} items={items} />;
}

async function HomeCollectionsRail() {
  // Server-side access control already filters out collections whose
  // members all live in libraries this user can't see — so we don't
  // need a separate visible-lib intersection here.
  let collections;
  try {
    const r = await collectionsApi.list();
    collections = r.collections;
  } catch {
    return null;
  }
  if (collections.length === 0) return null;
  return <CollectionsRail collections={collections} />;
}

async function GenreRail({
  libraryId,
  kind,
  genre,
}: {
  libraryId: number;
  kind: "movie" | "show";
  genre: string;
}) {
  const res = await itemsApi.list({
    library_id: libraryId,
    kind,
    genre,
    page_size: 16,
  });
  const items = res.items.map(adaptItem);
  if (items.length < 4) return null;
  return (
    <Rail
      title={genre}
      items={items}
      href={`/genre/${encodeURIComponent(genre)}`}
    />
  );
}
