import { redirect } from "next/navigation";
import { Suspense } from "react";
import { CollectionsRail } from "@/components/CollectionsRail";
import { EmptyHomeClient } from "@/components/EmptyHomeClient";
import { Hero } from "@/components/Hero";
import { ModalRoot } from "@/components/ModalRoot";
import { Rail } from "@/components/Rail";
import { RailErrorBoundary } from "@/components/RailErrorBoundary";
import { HeroSkeleton, RailSkeleton } from "@/components/Skeleton";
import { Top10Rail } from "@/components/Top10Rail";
import { pickHeroIndex } from "@/lib/hero";
import {
  ChimpFlixApiError,
  admin as adminApi,
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
  const user = await requireUser("/");
  // First-run wizard auto-redirect. Owners/admins hitting Home on a
  // fresh install would otherwise land on empty rails; bounce them
  // into `/onboarding` instead. Viewers stay on Home — they can't
  // add libraries anyway. The wizard flips `setup_completed = true`
  // on finish or skip so this redirect only fires once.
  if (user.role === "owner" || user.role === "admin") {
    try {
      const { settings } = await adminApi.settings.get();
      if (!settings.setup_completed) {
        redirect("/onboarding");
      }
    } catch (e) {
      // 403 from /admin/settings means the user doesn't actually
      // have admin scope (e.g. role downgraded mid-session). Fall
      // through to normal Home rather than block.
      if (!(e instanceof ChimpFlixApiError && e.status === 403)) {
        throw e;
      }
    }
  }
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

  // Empty-Home probe: a freshly-deployed instance with libraries
  // configured but nothing scanned yet would otherwise render the
  // full rail tree against an empty index, producing a black page
  // with no explanation. A single `items.list(page_size=1)` returns
  // `total` cheaply; if it's zero we swap in a friendly card with
  // live scan progress. Wrap in try/catch so an items-API hiccup
  // never breaks Home — fall through to the normal render and let
  // the per-rail Suspense boundaries surface their own errors.
  //
  // When `visibleLibIds` is empty (operator has libraries but they're
  // all visibility=hidden, or hasn't created any) we short-circuit to
  // the empty state without hitting items.list — the backend treats
  // an empty `library_ids` as "no filter, return everything" which
  // would mask the empty state.
  let itemTotal: number | null = null;
  if (visibleLibIds.length === 0) {
    itemTotal = 0;
  } else {
    try {
      const probe = await itemsApi.list({
        library_ids: visibleLibIds,
        page_size: 1,
      });
      itemTotal = probe.total;
    } catch {
      itemTotal = null;
    }
  }
  const isFreshlyEmpty = itemTotal === 0;

  if (isFreshlyEmpty) {
    return (
      <main className="relative min-h-screen">
        <EmptyHomeClient
          libraries={libs}
          isAdmin={user.role === "owner" || user.role === "admin"}
        />
      </main>
    );
  }

  return (
    <main className="relative">
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
    // Dedupe across libraries (same title in /anime and /movies
    // would otherwise render twice) and re-rank to 1..N. The upstream
    // ranks are global TMDB popularity; once we intersect with the
    // local library we'd see gaps (3, 4, 5, 7, …) — Netflix shows a
    // clean 1, 2, 3 sequence, and relative order is what matters.
    const seen = new Set<number>();
    const unique = res.items.filter((it) => {
      if (it.tmdb_id == null) return true;
      if (seen.has(it.tmdb_id)) return false;
      seen.add(it.tmdb_id);
      return true;
    });
    entries = unique.map(({ rank: _rank, ...item }, idx) => ({
      rank: idx + 1,
      item: adaptItem(item),
    }));
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
