// App service worker.
//
// Goals:
//   - Survive HTTP cache eviction. The browser's disk cache can drop entries
//     under pressure; a SW cache is more durable.
//   - Instant repeat-visit paint. Nav HTML is served from cache while the
//     network revalidates in the background.
//   - Programmatic cache control: cache versioning, deliberate eviction
//     when assets change.
//
// Bump CACHE_VERSION whenever response shapes change in a way that would
// poison cached entries — old caches get purged on activate.

// Bumped from v1 → v2 to evict caches that may have stored the
// unauthenticated home-page redirect-to-login (Next.js returns it as a
// 200 with an embedded `<meta refresh>` because the redirect() call
// happens inside a streaming Suspense boundary). Old entries served
// from cache would meta-refresh signed-in users back to /login.
const CACHE_VERSION = "app-v2";
const STATIC_CACHE = `${CACHE_VERSION}-static`;
const IMAGE_CACHE = `${CACHE_VERSION}-images`;
const PAGE_CACHE = `${CACHE_VERSION}-pages`;
const ALL_CACHES = [STATIC_CACHE, IMAGE_CACHE, PAGE_CACHE];

// Routes worth pre-fetching on SW install so the first navigation to them
// is served from cache. Skip API and dynamic routes here — those get
// populated on-demand.
const PRECACHE_ROUTES = ["/", "/movies", "/shows", "/new-popular", "/my-list"];

self.addEventListener("install", (event) => {
  event.waitUntil(
    (async () => {
      // Best-effort precache. Failures are non-fatal — pages still work,
      // they just pay a normal first-visit cost.
      const cache = await caches.open(PAGE_CACHE);
      await Promise.all(
        PRECACHE_ROUTES.map(async (route) => {
          try {
            const resp = await fetch(route, { credentials: "include" });
            if (isCacheable(resp)) await cache.put(route, resp);
          } catch {
            // ignore
          }
        }),
      );
      // skipWaiting + clientsClaim let the new SW take over immediately
      // instead of waiting for all old tabs to close.
      await self.skipWaiting();
    })(),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const names = await caches.keys();
      await Promise.all(
        names
          .filter((n) => !ALL_CACHES.includes(n))
          .map((n) => caches.delete(n)),
      );
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;

  // Auth and pref endpoints must always be live — never cache user state.
  if (url.pathname.startsWith("/api/auth/")) return;
  if (url.pathname.startsWith("/api/prefs/")) return;
  if (url.pathname.startsWith("/api/modal/")) return;
  if (url.pathname.startsWith("/api/tmdb/")) return;
  // Plex JSON endpoints (everything under /api/plex/* that ISN'T the photo
  // proxy) are token-scoped — leave them to the network + our in-memory
  // server cache.
  if (
    url.pathname.startsWith("/api/plex/") &&
    !url.pathname.startsWith("/api/plex/photo/")
  ) {
    return;
  }
  // HLS streams: never cache. The transcoder URLs include short-lived
  // session params and segment data is too big for SW caches anyway.
  if (
    url.pathname.includes("/transcode/universal/") ||
    url.pathname.endsWith(".m3u8") ||
    url.pathname.endsWith(".ts")
  ) {
    return;
  }

  // Plex image proxy: cache-first. Plex art URLs are versioned in their
  // path, so a given URL never changes content. Survives forever in this
  // cache, evicted only by version bump.
  if (url.pathname.startsWith("/api/plex/photo/")) {
    event.respondWith(cacheFirst(req, IMAGE_CACHE));
    return;
  }

  // Static Next.js assets: cache-first. Hashed filenames mean URLs change
  // when bundles change — old entries become unreachable and get cleaned
  // up on the next version bump.
  if (url.pathname.startsWith("/_next/static/")) {
    event.respondWith(cacheFirst(req, STATIC_CACHE));
    return;
  }

  // Document navigations: stale-while-revalidate. User sees the cached
  // page shell instantly; fresh content lands a beat later when the
  // network revalidates. The streaming RSC chunks make this safe — even
  // a stale shell renders skeletons for any data that streams in.
  if (req.mode === "navigate") {
    event.respondWith(staleWhileRevalidate(req, PAGE_CACHE));
    return;
  }
});

// Decide whether a response is safe to store in the SW cache. Honors
// `Cache-Control: no-store` so the unauthenticated home page — which
// Next.js returns as HTTP 200 with an embedded `<meta refresh>`
// redirect when redirect() is called from inside a streaming Suspense
// boundary — never lands in PAGE_CACHE. Without this, a cached redirect
// page would meta-refresh signed-in users back to /login on every
// repeat visit.
function isCacheable(resp) {
  if (!resp || !resp.ok) return false;
  const cc = resp.headers.get("Cache-Control") || "";
  if (/\bno-store\b/i.test(cc)) return false;
  return true;
}

async function cacheFirst(req, cacheName) {
  const cache = await caches.open(cacheName);
  const cached = await cache.match(req);
  if (cached) {
    // Background-refresh stale-but-served entries occasionally so the cache
    // doesn't drift on long-running PWA sessions.
    refreshIfStale(req, cache).catch(() => {});
    return cached;
  }
  try {
    const resp = await fetch(req);
    if (isCacheable(resp)) {
      // Clone before storing — the response body is a stream and can only
      // be consumed once.
      cache.put(req, resp.clone()).catch(() => {});
    }
    return resp;
  } catch (err) {
    // Surface the network error if there's nothing cached. Caller decides.
    if (cached) return cached;
    throw err;
  }
}

async function staleWhileRevalidate(req, cacheName) {
  const cache = await caches.open(cacheName);
  const cached = await cache.match(req);
  const networkPromise = fetch(req)
    .then((resp) => {
      if (isCacheable(resp)) cache.put(req, resp.clone()).catch(() => {});
      return resp;
    })
    .catch(() => cached || Response.error());
  return cached ?? networkPromise;
}

// One-day refresh interval for cache-first entries. Keeps long-tail entries
// fresh-ish without thrashing on every hit.
const STALE_AFTER_MS = 24 * 60 * 60 * 1000;

async function refreshIfStale(req, cache) {
  const cached = await cache.match(req);
  if (!cached) return;
  const dateHeader = cached.headers.get("date");
  if (!dateHeader) return;
  const age = Date.now() - new Date(dateHeader).getTime();
  if (age < STALE_AFTER_MS) return;
  try {
    const resp = await fetch(req);
    if (isCacheable(resp)) cache.put(req, resp.clone());
  } catch {
    // ignore — stale entry stays valid
  }
}

// Allow the page to message the SW: SKIP_WAITING forces activation,
// CLEAR_CACHES nukes everything (handy for debugging / sign-out).
self.addEventListener("message", (event) => {
  if (!event.data) return;
  if (event.data.type === "SKIP_WAITING") {
    self.skipWaiting();
  } else if (event.data.type === "CLEAR_CACHES") {
    event.waitUntil(
      (async () => {
        const names = await caches.keys();
        await Promise.all(names.map((n) => caches.delete(n)));
      })(),
    );
  }
});
