// Background warmer for the Plex data cache.
//
// Lazy-bootstrapped on the first authenticated request that supplies a
// ServerAuth (server URL + access token). From then on, a setInterval
// loop refetches all the global rails (sections, per-section new/top/
// top-rated, per-genre lists for the first movie/show library) every
// WARM_INTERVAL_MS.
//
// Pages render against the cache populated by this loop, so navigations
// don't pay Plex latency. Per-user data (onDeck, recentlyAdded) is still
// fetched on-demand inside Suspense boundaries — small (≤2 calls) and
// fast.

import {
  onDeck,
  recentlyAdded,
  sectionByGenre,
  sectionRecentlyAdded,
  sectionTopRated,
  sectionTopWatched,
  sections,
  type ServerAuth,
} from "./plex-data";

const WARM_INTERVAL_MS = 60_000;

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

let activeAuth: ServerAuth | null = null;
let timer: ReturnType<typeof setInterval> | null = null;
let inFlight: Promise<void> | null = null;

const WARM_CONCURRENCY = 6;

async function runWithConcurrency<T>(
  tasks: Array<() => Promise<T>>,
  limit: number,
): Promise<void> {
  let i = 0;
  async function worker() {
    while (i < tasks.length) {
      const idx = i++;
      try {
        await tasks[idx]();
      } catch {
        // Individual task failures don't stop the cycle. Cache helpers
        // already swallow + serve stale, so this is just defensive.
      }
    }
  }
  const workers = Array.from(
    { length: Math.min(limit, tasks.length) },
    () => worker(),
  );
  await Promise.all(workers);
}

async function warmAll(auth: ServerAuth): Promise<void> {
  if (inFlight) return inFlight;
  inFlight = (async () => {
    try {
      const libs = await sections(auth);
      const visibleLibs = libs.filter(
        (s) => s.type === "movie" || s.type === "show",
      );

      const tasks: Array<() => Promise<unknown>> = [];

      for (const lib of visibleLibs) {
        tasks.push(() => sectionRecentlyAdded(auth, lib.key));
        tasks.push(() => sectionTopWatched(auth, lib.key, 10));
        tasks.push(() => sectionTopRated(auth, lib.key, 24));
      }

      const firstMovie = visibleLibs.find((s) => s.type === "movie");
      const firstShow = visibleLibs.find((s) => s.type === "show");
      if (firstMovie) {
        for (const g of MOVIE_GENRES) {
          tasks.push(() => sectionByGenre(auth, firstMovie.key, g, 16));
        }
      }
      if (firstShow) {
        for (const g of SHOW_GENRES) {
          tasks.push(() => sectionByGenre(auth, firstShow.key, g, 16));
        }
      }

      tasks.push(() => recentlyAdded(auth));
      tasks.push(() => onDeck(auth));

      await runWithConcurrency(tasks, WARM_CONCURRENCY);
    } catch {
      // Best-effort; next tick retries.
    } finally {
      inFlight = null;
    }
  })();
  return inFlight;
}

/**
 * Idempotent — call from any authenticated request that has a server
 * auth. The first call kicks off the warmer; subsequent calls are no-ops
 * as long as the (server id, access token) tuple hasn't changed. If
 * either changes (server switch, profile switch), the warmer restarts
 * targeting the new pair.
 */
export function ensureWarmerStarted(auth: ServerAuth): void {
  if (
    activeAuth &&
    activeAuth.id === auth.id &&
    activeAuth.accessToken === auth.accessToken &&
    activeAuth.url === auth.url
  ) {
    return;
  }
  activeAuth = auth;

  if (timer) clearInterval(timer);
  warmAll(auth).catch(() => {});
  timer = setInterval(() => {
    if (activeAuth) warmAll(activeAuth).catch(() => {});
  }, WARM_INTERVAL_MS);
}
