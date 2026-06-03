import Link from "next/link";
import {
  trakt as traktApi,
  items as itemsApi,
  type TraktUpcomingMovie,
} from "@/lib/chimpflix-api";
import { plexImage } from "@/lib/image";

// Cap SSR fan-out so a large Trakt calendar doesn't fire dozens of parallel
// item-lookup requests before the page can stream any HTML.
const SSR_FETCH_CONCURRENCY = 6;

async function throttledAll<T>(
  tasks: (() => Promise<T>)[],
  limit: number,
): Promise<T[]> {
  const results: T[] = new Array(tasks.length);
  let next = 0;
  async function worker() {
    while (next < tasks.length) {
      const i = next++;
      results[i] = await tasks[i]();
    }
  }
  const workers = Array.from({ length: Math.min(limit, tasks.length) }, worker);
  await Promise.all(workers);
  return results;
}

function formatAirRelative(firstAired: string, now: Date): string {
  const air = new Date(firstAired);
  const ms = air.getTime() - now.getTime();
  const days = Math.round(ms / (24 * 60 * 60 * 1000));
  if (days < 0) return "aired";
  if (days === 0) return "today";
  if (days === 1) return "tomorrow";
  if (days < 7) return `in ${days} days`;
  if (days < 14) return "next week";
  return air.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function episodeCode(season: number, episode: number): string {
  const s = String(season).padStart(2, "0");
  const e = String(episode).padStart(2, "0");
  return `S${s}E${e}`;
}

/// "Coming Soon" rail — upcoming episodes for shows the user has
/// watched on Trakt, hydrated with the local item so each tile
/// click-throughs into the title modal for shows we have. Entries
/// without a matching local show are dropped (no clickable target,
/// no poster URL we trust).
///
/// Variant selector controls which Trakt calendar feeds the rail:
///   - "shows" (default): every upcoming episode of every tracked show
///   - "premieres":       season premieres only — S(N+1)E1
///   - "new":             series premieres — E1 of brand-new shows
///
/// Renders nothing when the user hasn't linked Trakt or no upcoming
/// episodes match local content.
export async function ComingSoonRail({
  variant = "shows",
  title = "Coming Soon",
  days = 14,
}: {
  variant?: "shows" | "premieres" | "new";
  title?: string;
  days?: number;
} = {}) {
  let upcoming;
  try {
    upcoming = await traktApi.calendarShows(days, variant);
  } catch {
    // Trakt 401 / network blip — silent no-op rather than blocking the
    // page render. Other Trakt-driven UI (Sync now) already surfaces
    // hard errors.
    return null;
  }
  const matched = upcoming.items.filter(
    (e) => e.show_item_id !== null && e.show_item_id !== undefined,
  );
  if (matched.length === 0) return null;
  // Dedup by show — one tile per show, the soonest upcoming episode.
  const seen = new Set<number>();
  const oneEachShow = matched.filter((e) => {
    const id = e.show_item_id as number;
    if (seen.has(id)) return false;
    seen.add(id);
    return true;
  });
  // Fetch the show items with bounded concurrency so SSR doesn't fire an
  // unbounded fan-out before the page can stream HTML.
  const shows = await throttledAll(
    oneEachShow.map((e) => () =>
      itemsApi.get(e.show_item_id as number).catch(() => null),
    ),
    SSR_FETCH_CONCURRENCY,
  );
  const now = new Date();
  const tiles = oneEachShow
    .map((entry, idx) => ({ entry, show: shows[idx] }))
    .filter(({ show }) => show !== null);
  if (tiles.length === 0) return null;
  return (
    <section
      className="zf-rise-in px-4 pb-1 pt-1 sm:px-8 md:px-12"
      style={{
        contentVisibility: "auto",
        containIntrinsicSize: "260px",
      }}
    >
      <h2 className="mb-3 text-lg font-semibold tracking-tight sm:text-xl md:text-[1.4rem]">
        {title}
      </h2>
      <div className="flex gap-3 overflow-x-auto overscroll-x-contain touch-pan-x pb-2 sm:gap-4">
        {tiles.map(({ entry, show }) => {
          if (!show) return null;
          const thumbPath =
            show.backdrop_path ?? show.poster_path ?? undefined;
          const img = plexImage(thumbPath ?? undefined, 480, 270);
          const when = formatAirRelative(entry.first_aired, now);
          return (
            <Link
              key={`${entry.show_item_id}-${entry.season}-${entry.episode}`}
              href={`/watch/${entry.show_item_id}`}
              className="group relative flex-none overflow-hidden rounded-md bg-neutral-900 transition-transform hover:scale-[1.04] focus:scale-[1.04] focus:outline-none focus-visible:ring-2 focus-visible:ring-accent"
              style={{ width: 320 }}
            >
              {img && (
                // eslint-disable-next-line @next/next/no-img-element
                <img
                  src={img}
                  alt={show.title}
                  width={320}
                  height={180}
                  loading="lazy"
                  className="block h-45 w-80 object-cover"
                />
              )}
              <div className="absolute inset-x-0 bottom-0 bg-linear-to-t from-black/90 to-transparent p-3">
                <div className="line-clamp-1 text-sm font-semibold">
                  {show.title}
                </div>
                <div className="text-xs text-neutral-300">
                  {episodeCode(entry.season, entry.episode)}
                  {entry.episode_title ? ` — ${entry.episode_title}` : ""}
                </div>
                <div className="text-[11px] uppercase tracking-wide text-red-400">
                  {when}
                </div>
              </div>
            </Link>
          );
        })}
      </div>
    </section>
  );
}

/// Companion to [`ComingSoonRail`] for movie releases — driven by
/// `/calendars/my/movies` (the user's watchlist + collection on Trakt).
/// Same rail visual, different fetch + tile content. Default window is
/// 30 days since theatrical / streaming releases are more sparse than
/// weekly episodes.
export async function UpcomingMoviesRail({
  days = 30,
}: { days?: number } = {}) {
  let upcoming;
  try {
    upcoming = await traktApi.calendarMovies(days);
  } catch {
    return null;
  }
  const matched = upcoming.items.filter(
    (m: TraktUpcomingMovie) =>
      m.movie_item_id !== null && m.movie_item_id !== undefined,
  );
  if (matched.length === 0) return null;
  // Same bounded fan-out as ComingSoonRail — 30-day movie window can
  // accumulate many more entries than the show calendar.
  const movies = await throttledAll(
    matched.map((m) => () =>
      itemsApi.get(m.movie_item_id as number).catch(() => null),
    ),
    SSR_FETCH_CONCURRENCY,
  );
  const tiles = matched
    .map((entry, idx) => ({ entry, movie: movies[idx] }))
    .filter(({ movie }) => movie !== null);
  if (tiles.length === 0) return null;
  const now = new Date();
  return (
    <section
      className="zf-rise-in px-4 pb-1 pt-1 sm:px-8 md:px-12"
      style={{ contentVisibility: "auto", containIntrinsicSize: "260px" }}
    >
      <h2 className="mb-3 text-lg font-semibold tracking-tight sm:text-xl md:text-[1.4rem]">
        Upcoming Movies
      </h2>
      <div className="flex gap-3 overflow-x-auto overscroll-x-contain touch-pan-x pb-2 sm:gap-4">
        {tiles.map(({ entry, movie }) => {
          if (!movie) return null;
          const thumbPath =
            movie.backdrop_path ?? movie.poster_path ?? undefined;
          const img = plexImage(thumbPath ?? undefined, 480, 270);
          // Trakt sends `released` as YYYY-MM-DD with no time; pin to
          // local midnight UTC so the day-count math doesn't drift by
          // a day in the user's tz.
          const when = formatAirRelative(`${entry.released}T00:00:00Z`, now);
          return (
            <Link
              key={`${entry.movie_item_id}-${entry.released}`}
              href={`/watch/${entry.movie_item_id}`}
              className="group relative flex-none overflow-hidden rounded-md bg-neutral-900 transition-transform hover:scale-[1.04] focus:scale-[1.04] focus:outline-none focus-visible:ring-2 focus-visible:ring-accent"
              style={{ width: 320 }}
            >
              {img && (
                // eslint-disable-next-line @next/next/no-img-element
                <img
                  src={img}
                  alt={movie.title}
                  width={320}
                  height={180}
                  loading="lazy"
                  className="block h-45 w-80 object-cover"
                />
              )}
              <div className="absolute inset-x-0 bottom-0 bg-linear-to-t from-black/90 to-transparent p-3">
                <div className="line-clamp-1 text-sm font-semibold">
                  {movie.title}
                  {entry.year ? ` (${entry.year})` : ""}
                </div>
                <div className="text-[11px] uppercase tracking-wide text-red-400">
                  {when}
                </div>
              </div>
            </Link>
          );
        })}
      </div>
    </section>
  );
}
