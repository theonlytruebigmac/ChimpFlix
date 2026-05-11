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

// Whether the warmer has completed its first full cycle. Stashed on
// globalThis so a single Node process shares this between every Next.js
// route bundle — Turbopack code-splits each route into its own bundle
// with its own module copies, so a plain `let firstTickDone` here would
// give /api/warmer-status its own private (and permanently-false) view
// of warmer state. Process-wide global is the cheapest "actually shared
// state" we can get.
//
// The first tick uses TTL-respecting fetches (the cache is empty so
// getOrFetch fetches fresh anyway, and we don't want to pile force-
// refreshed Plex calls on top of the initial cold-start user requests).
// Subsequent ticks force-refresh so entries get renewed before TTL
// expiry, keeping every user-facing read on the cache-hit path.
type WarmerState = { firstTickDone: boolean };
const g = globalThis as typeof globalThis & { __cfWarmerState?: WarmerState };
if (!g.__cfWarmerState) g.__cfWarmerState = { firstTickDone: false };
const warmerState = g.__cfWarmerState;

/**
 * Has the warmer completed at least one full warmAll cycle? Used to
 * gate the "Preparing your library…" cold-start overlay — once true,
 * the global rails are populated and page renders should hit cache
 * on the hot path.
 */
export function isWarmerReady(): boolean {
  return warmerState.firstTickDone;
}

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

// Concurrent Plex requests per warmer tick. Set conservatively because
// Plex's library-section endpoints contend on a shared DB connection and
// degrade non-linearly above ~4 concurrent requests — we saw 17–34s per
// call when running at 6, vs ~1–2s at 3. The page itself fires its own
// concurrent rails on render, so a quiet warmer leaves more headroom
// for user-driven fetches.
const WARM_CONCURRENCY = 3;

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
  // First tick (cold cache) uses TTL-respecting fetches — cache is empty
  // anyway and we don't want to redundantly force-refresh entries the
  // user's request is already populating. Subsequent ticks force-refresh
  // so entries get renewed *before* their TTL expires, keeping every
  // user-facing read on the cache-hit path.
  const forceRefresh = warmerState.firstTickDone;
  const tStart = Date.now();
  console.log(`[warmer] tick start force=${forceRefresh}`);
  inFlight = (async () => {
    try {
      const libs = await sections(auth);
      const visibleLibs = libs.filter(
        (s) => s.type === "movie" || s.type === "show",
      );

      const tasks: Array<() => Promise<unknown>> = [];

      for (const lib of visibleLibs) {
        tasks.push(() =>
          sectionRecentlyAdded(auth, lib.key, { forceRefresh }),
        );
        tasks.push(() =>
          sectionTopWatched(auth, lib.key, 10, { forceRefresh }),
        );
        tasks.push(() =>
          sectionTopRated(auth, lib.key, 24, { forceRefresh }),
        );
      }

      const firstMovie = visibleLibs.find((s) => s.type === "movie");
      const firstShow = visibleLibs.find((s) => s.type === "show");
      if (firstMovie) {
        for (const g of MOVIE_GENRES) {
          tasks.push(() =>
            sectionByGenre(auth, firstMovie.key, g, 16, { forceRefresh }),
          );
        }
      }
      if (firstShow) {
        for (const g of SHOW_GENRES) {
          tasks.push(() =>
            sectionByGenre(auth, firstShow.key, g, 16, { forceRefresh }),
          );
        }
      }

      tasks.push(() => recentlyAdded(auth, { forceRefresh }));
      tasks.push(() => onDeck(auth, { forceRefresh }));

      await runWithConcurrency(tasks, WARM_CONCURRENCY);
      console.log(
        `[warmer] tick done in ${Date.now() - tStart}ms tasks=${tasks.length}`,
      );
    } catch (e) {
      console.log(
        `[warmer] tick errored after ${Date.now() - tStart}ms: ${e instanceof Error ? e.message : String(e)}`,
      );
      // Best-effort; next tick retries.
    } finally {
      inFlight = null;
      warmerState.firstTickDone = true;
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
  // Reset the "first tick" flag — a server / profile switch means the
  // new auth's cache is empty, same as cold-start. We want the first
  // tick under the new auth to be TTL-respecting, not force-refresh.
  warmerState.firstTickDone = false;

  if (timer) clearInterval(timer);
  warmAll(auth).catch(() => {});
  timer = setInterval(() => {
    if (activeAuth) warmAll(activeAuth).catch(() => {});
  }, WARM_INTERVAL_MS);
}
