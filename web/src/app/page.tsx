import { Suspense } from "react";
import { Hero } from "@/components/Hero";
import { ModalRoot } from "@/components/ModalRoot";
import { Rail } from "@/components/Rail";
import { HeroSkeleton, RailSkeleton } from "@/components/Skeleton";
import { Top10Rail } from "@/components/Top10Rail";
import { TopNav } from "@/components/TopNav";
import { pickHeroIndex } from "@/lib/hero";
import {
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
      <Suspense fallback={<HeroSkeleton />}>
        <HomeHero visibleLibIds={visibleLibIds} />
      </Suspense>
      <div className="relative z-20 space-y-1 pb-24 pt-4">
        <Suspense fallback={<RailSkeleton title="Continue Watching" />}>
          <ContinueWatchingRail />
        </Suspense>
        <Suspense fallback={<RailSkeleton title="Recently Added" />}>
          <RecentlyAddedRail visibleLibIds={visibleLibIds} />
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
        {libs.map((lib) => (
          <Suspense
            key={`lib-${lib.id}`}
            fallback={<RailSkeleton title={`New in ${lib.name}`} />}
          >
            <LibSectionRail lib={lib} />
          </Suspense>
        ))}
        {firstMovieLib &&
          MOVIE_GENRES.map((g) => (
            <Suspense key={`movie-genre-${g}`} fallback={null}>
              <GenreRail libraryId={firstMovieLib.id} kind="movie" genre={g} />
            </Suspense>
          ))}
        {firstShowLib &&
          SHOW_GENRES.map((g) => (
            <Suspense key={`show-genre-${g}`} fallback={null}>
              <GenreRail libraryId={firstShowLib.id} kind="show" genre={g} />
            </Suspense>
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
  const onDeck = deckRes.items
    .map(adaptOnDeck)
    .filter((it): it is MediaItem & { art: string } => Boolean(it.art));
  const recent = latest.items
    .map(adaptItem)
    .filter(
      (it): it is MediaItem & { art: string } =>
        Boolean(it.art) && (it.type === "movie" || it.type === "show"),
    );
  const pool = [...onDeck, ...recent].slice(0, 5);
  if (pool.length === 0) return null;
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
  try {
    const res = await itemsApi.trending(kind, 10, visibleLibIds);
    const entries = res.items
      .map(({ rank, ...item }) => ({ rank, item: adaptItem(item) }));
    if (entries.length === 0) return null;
    return <Top10Rail title={title} items={entries} />;
  } catch {
    return null;
  }
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
