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

const MOVIE_GENRES = [
  "Action",
  "Comedy",
  "Drama",
  "Thriller",
  "Sci-Fi",
  "Horror",
  "Romance",
  "Adventure",
  "Animation",
  "Documentary",
];

export default async function MoviesPage() {
  const t0 = Date.now();
  const auth = await requireServerAuth();
  const t1 = Date.now();
  const [hidden, allSections] = await Promise.all([
    readHiddenLibraries(),
    sections(auth),
  ]);
  console.log(
    `[perf] /movies auth=${t1 - t0}ms top-await(hidden+sections)=${Date.now() - t1}ms`,
  );
  const movieLibs = allSections.filter(
    (s) => s.type === "movie" && !hidden.has(s.key),
  );
  const firstMovieKey = movieLibs[0]?.key ?? null;

  if (movieLibs.length === 0) {
    return (
      <main className="relative min-h-screen bg-black">
        <TopNav />
        <div className="px-12 pb-24 pt-28">
          <h1 className="mb-3 text-4xl font-bold tracking-tight">Movies</h1>
          <p className="text-white/60">
            No movie libraries on your Plex server.
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
        <h1 className="text-3xl font-bold tracking-tight">Movies</h1>
        <GenresDropdown genres={MOVIE_GENRES} type="movie" />
      </div>
      <Suspense fallback={<HeroSkeleton />}>
        <MoviesHero auth={auth} hidden={hidden} />
      </Suspense>
      <div className="relative z-20 space-y-1 pb-24 pt-4">
        <Suspense fallback={<RailSkeleton title="Continue Watching" />}>
          <ContinueWatchingMovies auth={auth} hidden={hidden} />
        </Suspense>
        <Suspense fallback={<RailSkeleton title="Recently Added" />}>
          <RecentlyAddedMovies auth={auth} hidden={hidden} />
        </Suspense>
        {movieLibs.map((lib) => (
          <Suspense
            key={`lib-${lib.key}`}
            fallback={<RailSkeleton title={`Top 10 in ${lib.title}`} />}
          >
            <MovieLibRails auth={auth} lib={lib} />
          </Suspense>
        ))}
        {firstMovieKey &&
          MOVIE_GENRES.map((g) => (
            <Suspense key={`movie-genre-${g}`} fallback={null}>
              <GenreRail auth={auth} sectionKey={firstMovieKey} genre={g} />
            </Suspense>
          ))}
      </div>
      <ModalRoot />
    </main>
  );
}

async function MoviesHero({
  auth,
  hidden,
}: {
  auth: ServerAuth;
  hidden: Set<string>;
}) {
  const t0 = Date.now();
  const [cw, latest] = await Promise.all([onDeck(auth), recentlyAdded(auth)]);
  console.log(`[perf] /movies MoviesHero onDeck+recent=${Date.now() - t0}ms`);
  const continueWatching = filterHiddenItems(cw, hidden).filter(
    (it) => it.type === "movie" && it.art,
  );
  const latestMovies = filterHiddenItems(latest, hidden).filter(
    (it) => it.type === "movie" && it.art,
  );
  const pool = [...continueWatching, ...latestMovies].slice(0, 5);
  if (pool.length === 0) return null;
  return <Hero item={pool[pickHeroIndex(pool, "movies")]} />;
}

async function ContinueWatchingMovies({
  auth,
  hidden,
}: {
  auth: ServerAuth;
  hidden: Set<string>;
}) {
  const t0 = Date.now();
  const items = filterHiddenItems(await onDeck(auth), hidden).filter(
    (it) => it.type === "movie",
  );
  console.log(
    `[perf] /movies ContinueWatchingMovies onDeck=${Date.now() - t0}ms`,
  );
  if (items.length === 0) return null;
  return <Rail title="Continue Watching" items={items} />;
}

async function RecentlyAddedMovies({
  auth,
  hidden,
}: {
  auth: ServerAuth;
  hidden: Set<string>;
}) {
  const t0 = Date.now();
  const items = filterHiddenItems(await recentlyAdded(auth), hidden).filter(
    (it) => it.type === "movie",
  );
  console.log(
    `[perf] /movies RecentlyAddedMovies recentlyAdded=${Date.now() - t0}ms`,
  );
  if (items.length === 0) return null;
  return <Rail title="Recently Added" items={items} />;
}

async function MovieLibRails({
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
    `[perf] /movies MovieLibRails(${lib.title}) recent+top=${Date.now() - t0}ms`,
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
    `[perf] /movies GenreRail(${genre}) sectionByGenre=${Date.now() - t0}ms`,
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
