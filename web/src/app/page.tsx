import { Suspense } from "react";
import { Hero } from "@/components/Hero";
import { ModalRoot } from "@/components/ModalRoot";
import { Rail } from "@/components/Rail";
import { HeroSkeleton, RailSkeleton } from "@/components/Skeleton";
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
  const libs = allLibs.filter((l) => !hidden.has(l.id));
  const firstMovieLib = libs.find((l) => l.kind === "movies");
  const firstShowLib = libs.find((l) => l.kind === "shows");

  return (
    <main className="relative">
      <TopNav />
      <Suspense fallback={<HeroSkeleton />}>
        <HomeHero />
      </Suspense>
      <div className="relative z-20 space-y-1 pb-24 pt-4">
        <Suspense fallback={<RailSkeleton title="Continue Watching" />}>
          <ContinueWatchingRail />
        </Suspense>
        <Suspense fallback={<RailSkeleton title="Recently Added" />}>
          <RecentlyAddedRail />
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

async function HomeHero() {
  const [deckRes, latest] = await Promise.all([
    playStateApi.onDeck(),
    itemsApi.list({ page_size: 12 }),
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

async function RecentlyAddedRail() {
  const res = await itemsApi.list({ page_size: RAIL_PAGE_SIZE });
  const items = res.items.map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title="Recently Added" items={items} />;
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
