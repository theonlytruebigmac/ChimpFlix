import { Suspense } from "react";
import { Hero } from "@/components/Hero";
import { ModalRoot } from "@/components/ModalRoot";
import { Rail } from "@/components/Rail";
import { ServerUnreachable } from "@/components/ServerUnreachable";
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

const MOVIE_GENRES = ["Action", "Comedy", "Drama"];
const SHOW_GENRES = ["Drama", "Comedy", "Animation"];

export default async function Home() {
  const auth = await requireServerAuth();
  // sections() is the canary call — if Plex is unreachable from here
  // (LAN URL not routable, server offline, TLS issue, etc.), every
  // other rail will fail too. Catch it once and render an actionable
  // error rather than letting the whole page crash.
  let allSections;
  let hidden: Set<string>;
  try {
    [hidden, allSections] = await Promise.all([
      readHiddenLibraries(),
      sections(auth),
    ]);
  } catch (e) {
    return <ServerUnreachable error={e} serverUrl={auth.url} />;
  }
  const libs = allSections.filter((s) => !hidden.has(s.key));
  const firstMovieKey = libs.find((s) => s.type === "movie")?.key ?? null;
  const firstShowKey = libs.find((s) => s.type === "show")?.key ?? null;

  return (
    <main className="relative">
      <TopNav />
      <Suspense fallback={<HeroSkeleton />}>
        <HomeHero auth={auth} hidden={hidden} />
      </Suspense>
      <div className="relative z-20 space-y-1 pb-24 pt-4">
        <Suspense fallback={<RailSkeleton title="Continue Watching" />}>
          <ContinueWatchingRail auth={auth} hidden={hidden} />
        </Suspense>
        <Suspense fallback={<RailSkeleton title="Recently Added" />}>
          <RecentlyAddedRail auth={auth} hidden={hidden} />
        </Suspense>
        {libs.map((lib) => (
          <Suspense
            key={`lib-${lib.key}`}
            fallback={<RailSkeleton title={`New in ${lib.title}`} />}
          >
            <LibSectionRails auth={auth} lib={lib} />
          </Suspense>
        ))}
        {firstMovieKey &&
          MOVIE_GENRES.map((g) => (
            <Suspense key={`movie-genre-${g}`} fallback={null}>
              <GenreRail auth={auth} sectionKey={firstMovieKey} genre={g} />
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

async function HomeHero({
  auth,
  hidden,
}: {
  auth: ServerAuth;
  hidden: Set<string>;
}) {
  const [cw, latest] = await Promise.all([onDeck(auth), recentlyAdded(auth)]);
  const continueWatching = filterHiddenItems(cw, hidden).filter((it) => it.art);
  const recentTitles = filterHiddenItems(latest, hidden).filter(
    (it) => it.art && (it.type === "movie" || it.type === "show"),
  );
  const pool = [...continueWatching, ...recentTitles].slice(0, 5);
  if (pool.length === 0) return null;
  return <Hero item={pool[pickHeroIndex(pool, "home")]} />;
}

async function ContinueWatchingRail({
  auth,
  hidden,
}: {
  auth: ServerAuth;
  hidden: Set<string>;
}) {
  const items = filterHiddenItems(await onDeck(auth), hidden);
  if (items.length === 0) return null;
  return <Rail title="Continue Watching" items={items} />;
}

async function RecentlyAddedRail({
  auth,
  hidden,
}: {
  auth: ServerAuth;
  hidden: Set<string>;
}) {
  const items = filterHiddenItems(await recentlyAdded(auth), hidden);
  if (items.length === 0) return null;
  return <Rail title="Recently Added" items={items} />;
}

async function LibSectionRails({
  auth,
  lib,
}: {
  auth: ServerAuth;
  lib: Section;
}) {
  const [newItems, topItems] = await Promise.all([
    sectionRecentlyAdded(auth, lib.key),
    sectionTopWatched(auth, lib.key, 12),
  ]);
  return (
    <>
      {newItems.length > 0 && (
        <Rail title={`New in ${lib.title}`} items={newItems} />
      )}
      {topItems.length > 0 && (
        <Rail title={`Top in ${lib.title}`} items={topItems} />
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
  const items = await sectionByGenre(auth, sectionKey, genre, 16);
  if (items.length < 4) return null;
  return (
    <Rail
      title={genre}
      items={items}
      href={`/genre/${encodeURIComponent(genre)}`}
    />
  );
}
