// Process-wide in-memory cache for Plex API responses.
//
// Purpose: avoid hitting Plex on every page render. Pages now read from this
// cache (via plex-data helpers), and a background warmer keeps the global
// rails fresh on a tick. The cache lives in module scope so it's shared
// across all requests within the same Node process.
//
// Lifecycle:
//   - Survives navigations and concurrent requests (it's just a Map).
//   - Dies on container restart (no persistence). The warmer rebuilds it.
//   - In dev with HMR, the module reloads on save and the cache is reset —
//     fine, since dev cold starts are tolerable.
//
// Stale-on-error: if a refresh fails and we have a previous value, we serve
// the stale one rather than throwing. Plex blips don't take the page down.

type Entry<T> = { data: T; updatedAt: number };

const store = new Map<string, Entry<unknown>>();
const inFlight = new Map<string, Promise<unknown>>();

export function get<T>(key: string): T | null {
  const e = store.get(key) as Entry<T> | undefined;
  return e ? e.data : null;
}

export function set<T>(key: string, data: T): void {
  store.set(key, { data, updatedAt: Date.now() });
}

export function has(key: string): boolean {
  return store.has(key);
}

export function size(): number {
  return store.size;
}

/**
 * Returns the cached value if it's within `ttlMs`. Otherwise calls `fetcher`,
 * stores the result, and returns it. Concurrent calls for the same key are
 * coalesced into a single fetch. On fetch failure, falls back to the
 * previous (stale) value if any.
 */
export async function getOrFetch<T>(
  key: string,
  fetcher: () => Promise<T>,
  opts: { ttlMs: number } = { ttlMs: 60_000 },
): Promise<T> {
  const now = Date.now();
  const e = store.get(key) as Entry<T> | undefined;
  if (e && now - e.updatedAt < opts.ttlMs) return e.data;

  const existing = inFlight.get(key) as Promise<T> | undefined;
  if (existing) return existing;

  const p = (async () => {
    try {
      const data = await fetcher();
      store.set(key, { data, updatedAt: Date.now() });
      return data;
    } catch (err) {
      if (e) return e.data;
      throw err;
    } finally {
      inFlight.delete(key);
    }
  })();
  inFlight.set(key, p);
  return p;
}

/**
 * Force-refresh a key by ignoring its TTL. Used by the warmer's tick. If a
 * refresh is already in flight, latches onto that.
 */
export async function refresh<T>(
  key: string,
  fetcher: () => Promise<T>,
): Promise<T> {
  const existing = inFlight.get(key) as Promise<T> | undefined;
  if (existing) return existing;
  const p = (async () => {
    try {
      const data = await fetcher();
      store.set(key, { data, updatedAt: Date.now() });
      return data;
    } finally {
      inFlight.delete(key);
    }
  })();
  inFlight.set(key, p);
  return p;
}
