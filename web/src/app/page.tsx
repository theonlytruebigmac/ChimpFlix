import { redirect } from "next/navigation";
import { Fragment, type ReactNode, Suspense } from "react";
import { CalendarRail } from "@/components/CalendarRail";
import { CollectionsRail } from "@/components/CollectionsRail";
import {
  ComingSoonRail,
  UpcomingMoviesRail,
} from "@/components/ComingSoonRail";
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
  trakt as traktApi,
  type Library,
} from "@/lib/chimpflix-api";
import { adaptItem, adaptOnDeck } from "@/lib/chimpflix-adapt";
import { requireUser } from "@/lib/chimpflix-server";
import type { MediaItem } from "@/lib/chimpflix-types";

const RAIL_PAGE_SIZE = 20;
const MOVIE_GENRES = ["Action", "Comedy", "Drama"];
const SHOW_GENRES = ["Drama", "Comedy", "Animation"];

// Stable rail-id catalogue, in default top-to-bottom order. MUST stay in
// sync with the backend `HOME_RAIL_CATALOGUE` (crates/library/src/models.rs)
// — that constant is the source of truth the prefs API validates against;
// this array maps each id to the rendered rail node so a user's
// `home_rails_json` overlay can drop/reorder them. Grouped rails
// (`library_sections`, `movie_genres`, `show_genres`) toggle as one unit.
const HOME_RAIL_ORDER = [
  "continue_watching",
  "recently_added",
  "coming_soon",
  "season_premieres",
  "calendar",
  "upcoming_movies",
  "trakt_recs_movies",
  "trakt_recs_shows",
  "trakt_favorites",
  "trakt_lists",
  "top10_movies",
  "top10_shows",
  "collections",
  "library_sections",
  "movie_genres",
  "show_genres",
] as const;

type HomeRailId = (typeof HOME_RAIL_ORDER)[number];

/// Parse the user's `home_rails_json` overlay into an id→enabled map plus the
/// user-desired order, or `null` when the user hasn't customized anything
/// (empty array / unparseable). A `null` result is the signal that the home
/// page renders its byte-for-byte default tree — no reorder logic runs at all.
function parseRailOverlay(
  raw: string,
): { enabled: Map<string, boolean>; order: string[] } | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!Array.isArray(parsed) || parsed.length === 0) return null;
  const enabled = new Map<string, boolean>();
  const order: string[] = [];
  for (const entry of parsed) {
    if (
      entry &&
      typeof entry === "object" &&
      typeof (entry as { rail_id?: unknown }).rail_id === "string"
    ) {
      const id = (entry as { rail_id: string }).rail_id;
      // Ignore ids we don't render (forward-compat: a future rail removed
      // from the frontend but still in someone's saved overlay).
      if (!(HOME_RAIL_ORDER as readonly string[]).includes(id)) continue;
      const on = (entry as { enabled?: unknown }).enabled !== false;
      enabled.set(id, on);
      order.push(id);
    }
  }
  if (order.length === 0) return null;
  return { enabled, order };
}

/// Apply the parsed overlay over the default catalogue order:
///   - rails the user explicitly disabled (`enabled === false`) are dropped;
///   - rails present in the overlay are placed first, in the user's order;
///   - rails absent from the overlay keep their default relative position,
///     appended after the user-ordered ones (sparse-overlay semantics).
function orderRailIds(overlay: {
  enabled: Map<string, boolean>;
  order: string[];
}): HomeRailId[] {
  const result: HomeRailId[] = [];
  const placed = new Set<string>();
  const isEnabled = (id: string) => overlay.enabled.get(id) !== false;
  // 1. User-ordered rails first (only valid catalogue ids, only enabled).
  for (const id of overlay.order) {
    if (placed.has(id)) continue;
    if (!(HOME_RAIL_ORDER as readonly string[]).includes(id)) continue;
    placed.add(id);
    if (isEnabled(id)) result.push(id as HomeRailId);
  }
  // 2. Remaining default rails keep their default order, dropping any the
  //    user disabled.
  for (const id of HOME_RAIL_ORDER) {
    if (placed.has(id)) continue;
    if (isEnabled(id)) result.push(id);
  }
  return result;
}

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
      // `count_only` makes this a pure existence check: the server returns
      // `total` with zero items AND bypasses the per-user kids_safe filter.
      // Without the bypass, a kids_safe profile on a library with no rated
      // items would see `total === 0` and a false "scan in progress" screen.
      // The probe returns no titles, so the bypass can't leak content.
      const probe = await itemsApi.list({
        library_ids: visibleLibIds,
        page_size: 1,
        count_only: true,
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

  // Per-user home customization (Feature 2). `parseRailOverlay` returns
  // `null` when the user hasn't customized their rails at all — the common
  // case — in which case we fall through to the byte-for-byte default tree
  // below, identical to the pre-feature home page.
  const railOverlay = parseRailOverlay(user.home_rails_json);

  // Each catalogue rail rendered exactly as in the default tree. Keyed by the
  // stable rail id so a customized layout can pick/drop/reorder them without
  // duplicating the JSX. NOTE: building this map does NOT trigger any data
  // fetch — these are inert React elements; the fetch only happens when (and
  // if) a node is actually rendered inside its Suspense boundary. So dropping
  // a rail genuinely skips its API call.
  const railNodes: Record<HomeRailId, ReactNode> = {
    continue_watching: (
      <RailErrorBoundary key="continue_watching" label="ContinueWatching">
        <Suspense fallback={<RailSkeleton title="Continue Watching" />}>
          <ContinueWatchingRail />
        </Suspense>
      </RailErrorBoundary>
    ),
    recently_added: (
      <RailErrorBoundary key="recently_added" label="RecentlyAdded">
        <Suspense fallback={<RailSkeleton title="Recently Added" />}>
          <RecentlyAddedRail visibleLibIds={visibleLibIds} />
        </Suspense>
      </RailErrorBoundary>
    ),
    coming_soon: (
      <RailErrorBoundary key="coming_soon" label="ComingSoon">
        <Suspense fallback={null}>
          <ComingSoonRail />
        </Suspense>
      </RailErrorBoundary>
    ),
    season_premieres: (
      <RailErrorBoundary key="season_premieres" label="SeasonPremieres">
        <Suspense fallback={null}>
          <ComingSoonRail variant="premieres" title="New Seasons" days={30} />
        </Suspense>
      </RailErrorBoundary>
    ),
    calendar: (
      <RailErrorBoundary key="calendar" label="Calendar">
        <Suspense fallback={null}>
          <CalendarRail visibleLibIds={visibleLibIds} />
        </Suspense>
      </RailErrorBoundary>
    ),
    upcoming_movies: (
      <RailErrorBoundary key="upcoming_movies" label="UpcomingMovies">
        <Suspense fallback={null}>
          <UpcomingMoviesRail />
        </Suspense>
      </RailErrorBoundary>
    ),
    trakt_recs_movies: (
      <RailErrorBoundary key="trakt_recs_movies" label="TraktRecsMovies">
        <Suspense fallback={null}>
          <TraktRecommendationsRail kind="movie" title="Recommended for You · Movies" />
        </Suspense>
      </RailErrorBoundary>
    ),
    trakt_recs_shows: (
      <RailErrorBoundary key="trakt_recs_shows" label="TraktRecsShows">
        <Suspense fallback={null}>
          <TraktRecommendationsRail kind="show" title="Recommended for You · Shows" />
        </Suspense>
      </RailErrorBoundary>
    ),
    trakt_favorites: (
      <RailErrorBoundary key="trakt_favorites" label="TraktFavorites">
        <Suspense fallback={null}>
          <TraktFavoritesRail />
        </Suspense>
      </RailErrorBoundary>
    ),
    trakt_lists: (
      <RailErrorBoundary key="trakt_lists" label="TraktLists">
        <Suspense fallback={null}>
          <TraktListsRails />
        </Suspense>
      </RailErrorBoundary>
    ),
    top10_movies: (
      <RailErrorBoundary key="top10_movies" label="Top10Movies">
        <Suspense fallback={null}>
          <Top10TrendingRail
            kind="movie"
            title="Top 10 Movies This Week"
            visibleLibIds={visibleLibIds}
          />
        </Suspense>
      </RailErrorBoundary>
    ),
    top10_shows: (
      <RailErrorBoundary key="top10_shows" label="Top10Shows">
        <Suspense fallback={null}>
          <Top10TrendingRail
            kind="show"
            title="Top 10 Shows This Week"
            visibleLibIds={visibleLibIds}
          />
        </Suspense>
      </RailErrorBoundary>
    ),
    collections: (
      <RailErrorBoundary key="collections" label="Collections">
        <Suspense fallback={<RailSkeleton title="Collections" />}>
          <HomeCollectionsRail />
        </Suspense>
      </RailErrorBoundary>
    ),
    library_sections: (
      <Fragment key="library_sections">
        {libs.map((lib) => (
          <RailErrorBoundary key={`lib-${lib.id}`} label={`Lib:${lib.name}`}>
            <Suspense fallback={<RailSkeleton title={`New in ${lib.name}`} />}>
              <LibSectionRail lib={lib} />
            </Suspense>
          </RailErrorBoundary>
        ))}
      </Fragment>
    ),
    movie_genres: (
      <Fragment key="movie_genres">
        {firstMovieLib &&
          MOVIE_GENRES.map((g) => (
            <RailErrorBoundary key={`movie-genre-${g}`} label={`MovieGenre:${g}`}>
              <Suspense fallback={null}>
                <GenreRail libraryId={firstMovieLib.id} kind="movie" genre={g} />
              </Suspense>
            </RailErrorBoundary>
          ))}
      </Fragment>
    ),
    show_genres: (
      <Fragment key="show_genres">
        {firstShowLib &&
          SHOW_GENRES.map((g) => (
            <RailErrorBoundary key={`show-genre-${g}`} label={`ShowGenre:${g}`}>
              <Suspense fallback={null}>
                <GenreRail libraryId={firstShowLib.id} kind="show" genre={g} />
              </Suspense>
            </RailErrorBoundary>
          ))}
      </Fragment>
    ),
  };

  // Default path (no overlay): render every rail in catalogue order. This is
  // identical to the pre-feature tree — same components, same props, same
  // order.
  const orderedIds: HomeRailId[] = railOverlay
    ? orderRailIds(railOverlay)
    : [...HOME_RAIL_ORDER];

  return (
    <main className="relative">
      <RailErrorBoundary label="HomeHero">
        <Suspense fallback={<HeroSkeleton />}>
          <HomeHero visibleLibIds={visibleLibIds} />
        </Suspense>
      </RailErrorBoundary>
      <div className="relative z-20 space-y-1 pb-24 pt-4">
        {orderedIds.map((id) => railNodes[id])}
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
  // On-deck already excludes finished titles server-side, so Continue
  // Watching never shows anything the user has completed — no client-side
  // "hide watched" filter is needed (the removed `hide_watched_cw` pref was
  // a guaranteed no-op against this list).
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

async function TraktFavoritesRail() {
  let res;
  try {
    res = await traktApi.favorites();
  } catch {
    return null;
  }
  const items = res.items.map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title="Your Trakt Favorites" items={items} />;
}

async function TraktListsRails() {
  let res;
  try {
    res = await traktApi.lists();
  } catch {
    return null;
  }
  if (res.lists.length === 0) return null;
  return (
    <>
      {res.lists.map((list) => {
        const items = list.items.map(adaptItem);
        if (items.length === 0) return null;
        return (
          <Rail
            key={`trakt-list-${list.id}`}
            title={list.name}
            items={items}
          />
        );
      })}
    </>
  );
}

async function TraktRecommendationsRail({
  kind,
  title,
}: {
  kind: "movie" | "show";
  title: string;
}) {
  let res;
  try {
    res = await traktApi.recommendations(kind);
  } catch {
    // Not linked / Trakt outage — silent no-op so the rail just
    // doesn't render. Other Trakt-driven UI handles hard errors.
    return null;
  }
  const items = res.items.map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title={title} items={items} />;
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
  return <Top10Rail title={title} items={entries} href="/new-popular" />;
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
