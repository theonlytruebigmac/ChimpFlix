import { Suspense } from "react";
import { GenresDropdown } from "@/components/GenresDropdown";
import { Hero } from "@/components/Hero";
import { ModalRoot } from "@/components/ModalRoot";
import { Rail } from "@/components/Rail";
import { HeroSkeleton, RailSkeleton } from "@/components/Skeleton";
import { TopNav } from "@/components/TopNav";
import { pickHeroIndex } from "@/lib/hero";
import {
  filterHiddenItems,
  readHiddenLibraries,
} from "@/lib/library-prefs";
import {
  onDeck,
  recentlyAdded,
  sectionByGenre,
  sectionRecentlyAdded,
  sectionTopWatched,
  sections,
  type Section,
  type ServerAuth,
} from "@/lib/plex-data";
import { requireServerAuth } from "@/lib/session";

const SHOW_GENRES = [
  "Drama",
  "Comedy",
  "Animation",
  "Crime",
  "Sci-Fi",
  "Action",
  "Thriller",
  "Documentary",
  "Family",
  "Reality",
];

export default async function ShowsPage() {
  const t0 = Date.now();
  const auth = await requireServerAuth();
  const t1 = Date.now();
  const [hidden, allSections] = await Promise.all([
    readHiddenLibraries(),
    sections(auth),
  ]);
  const t2 = Date.now();
  console.log(
    `[perf] /shows auth=${t1 - t0}ms top-await(hidden+sections)=${t2 - t1}ms`,
  );
  const showLibs = allSections.filter(
    (s) => s.type === "show" && !hidden.has(s.key),
  );
  const firstShowKey = showLibs[0]?.key ?? null;

  if (showLibs.length === 0) {
    return (
      <main className="relative min-h-screen bg-black">
        <TopNav />
        <div className="px-12 pb-24 pt-28">
          <h1 className="mb-3 text-4xl font-bold tracking-tight">TV Shows</h1>
          <p className="text-white/60">
            No show libraries on your Plex server.
          </p>
        </div>
        <ModalRoot />
      </main>
    );
  }

  return (
    <main className="relative">
      <TopNav />
      <div className="relative z-20 flex items-baseline gap-4 px-12 pt-24 pb-2">
        <h1 className="text-3xl font-bold tracking-tight">Shows</h1>
        <GenresDropdown genres={SHOW_GENRES} type="show" />
      </div>
      <Suspense fallback={<HeroSkeleton />}>
        <ShowsHero auth={auth} hidden={hidden} />
      </Suspense>
      <div className="relative z-20 space-y-1 pb-24 pt-4">
        <Suspense fallback={<RailSkeleton title="Continue Watching" />}>
          <ContinueWatchingShows auth={auth} hidden={hidden} />
        </Suspense>
        <Suspense fallback={<RailSkeleton title="Recently Added" />}>
          <RecentlyAddedShows auth={auth} hidden={hidden} />
        </Suspense>
        {showLibs.map((lib) => (
          <Suspense
            key={`lib-${lib.key}`}
            fallback={<RailSkeleton title={`Top 10 in ${lib.title}`} />}
          >
            <ShowLibRails auth={auth} lib={lib} />
          </Suspense>
        ))}
        {firstShowKey &&
          SHOW_GENRES.map((g) => (
            <Suspense key={`show-genre-${g}`} fallback={null}>
              <GenreRail auth={auth} sectionKey={firstShowKey} genre={g} />
            </Suspense>
          ))}
      </div>
      <ModalRoot />
    </main>
  );
}

async function ShowsHero({
  auth,
  hidden,
}: {
  auth: ServerAuth;
  hidden: Set<string>;
}) {
  const t0 = Date.now();
  const [cw, latest] = await Promise.all([onDeck(auth), recentlyAdded(auth)]);
  console.log(`[perf] /shows ShowsHero onDeck+recent=${Date.now() - t0}ms`);
  const continueWatching = filterHiddenItems(cw, hidden).filter(
    (it) => (it.type === "show" || it.type === "episode") && it.art,
  );
  const latestShows = filterHiddenItems(latest, hidden).filter(
    (it) =>
      (it.type === "show" ||
        it.type === "season" ||
        it.type === "episode") &&
      it.art,
  );
  const pool = [...continueWatching, ...latestShows].slice(0, 5);
  if (pool.length === 0) return null;
  return <Hero item={pool[pickHeroIndex(pool, "shows")]} />;
}

async function ContinueWatchingShows({
  auth,
  hidden,
}: {
  auth: ServerAuth;
  hidden: Set<string>;
}) {
  const t0 = Date.now();
  const items = filterHiddenItems(await onDeck(auth), hidden).filter(
    (it) => it.type === "show" || it.type === "episode",
  );
  console.log(`[perf] /shows ContinueWatchingShows onDeck=${Date.now() - t0}ms`);
  if (items.length === 0) return null;
  return <Rail title="Continue Watching" items={items} />;
}

async function RecentlyAddedShows({
  auth,
  hidden,
}: {
  auth: ServerAuth;
  hidden: Set<string>;
}) {
  const t0 = Date.now();
  const items = filterHiddenItems(await recentlyAdded(auth), hidden).filter(
    (it) =>
      it.type === "show" || it.type === "season" || it.type === "episode",
  );
  console.log(
    `[perf] /shows RecentlyAddedShows recentlyAdded=${Date.now() - t0}ms`,
  );
  if (items.length === 0) return null;
  return <Rail title="Recently Added" items={items} />;
}

async function ShowLibRails({
  auth,
  lib,
}: {
  auth: ServerAuth;
  lib: Section;
}) {
  const t0 = Date.now();
  const [newItems, topItems] = await Promise.all([
    sectionRecentlyAdded(auth, lib.key),
    sectionTopWatched(auth, lib.key, 10),
  ]);
  console.log(
    `[perf] /shows ShowLibRails(${lib.title}) recent+top=${Date.now() - t0}ms`,
  );
  return (
    <>
      {topItems.length >= 4 && (
        <Rail title={`Top 10 in ${lib.title}`} items={topItems} />
      )}
      {newItems.length >= 4 && (
        <Rail title={`New in ${lib.title}`} items={newItems} />
      )}
    </>
  );
}

async function GenreRail({
  auth,
  sectionKey,
  genre,
}: {
  auth: ServerAuth;
  sectionKey: string;
  genre: string;
}) {
  const t0 = Date.now();
  const items = await sectionByGenre(auth, sectionKey, genre, 16);
  console.log(
    `[perf] /shows GenreRail(${genre}) sectionByGenre=${Date.now() - t0}ms`,
  );
  if (items.length < 4) return null;
  return (
    <Rail
      title={genre}
      items={items}
      href={`/genre/${encodeURIComponent(genre)}`}
    />
  );
}
