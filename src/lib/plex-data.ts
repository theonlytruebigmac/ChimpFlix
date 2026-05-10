import { getOrFetch } from "./cache";
import { plexServer, type ServerAuth } from "./plex";
import {
  mapItem,
  type MediaItem,
  type MetadataNode,
  type Section,
} from "./plex-types";

export {
  displayTitle,
  formatRuntime,
  mapItem,
  type CastMember,
  type Marker,
  type MediaItem,
  type MetadataNode,
  type Section,
  type SearchHub,
} from "./plex-types";

export type { ServerAuth } from "./plex";

// Plex's list responses include full Media/Part/Stream blobs per item by
// default — codecs, audio/subtitle tracks, file paths, etc. None of that is
// needed to render cards, but for a 12-item list it can balloon to multiple
// megabytes per response. We strip those elements at the URL level for
// every list endpoint.
const SLIM_LIST = "&excludeElements=Media,Part,Stream";

// Cache TTLs picked for the trade-off between stale-vs-fresh:
//   - Per-user state (onDeck, recentlyAdded): 30s. Watch progress + new
//     files appear quickly enough that 30s lag is invisible.
//   - Section lists / metadata: 5 min. Stable enough that staleness is
//     rare; fresh enough that newly-added titles still appear within a
//     minute or two when the warmer's next tick lands.
//   - Sections themselves (library list): 1 hour.
const TTL_USER_STATE = 30_000;
const TTL_SECTION = 5 * 60_000;
const TTL_LIBRARIES = 60 * 60_000;

// Cache keys carry both the server ID and a token prefix so multi-tenant
// sessions don't accidentally serve User A's onDeck to User B (or worse,
// Server X's data when querying Server Y).
const tokenKey = (token: string) => token.slice(0, 12);
const k = (auth: ServerAuth, ...parts: string[]) =>
  [auth.id, ...parts].join(":");
const ku = (auth: ServerAuth, ...parts: string[]) =>
  [auth.id, tokenKey(auth.accessToken), ...parts].join(":");

async function fetchList(
  auth: ServerAuth,
  path: string,
): Promise<MediaItem[]> {
  const res = await plexServer(path, auth);
  if (!res.ok) return [];
  const data = await res.json();
  const items: MetadataNode[] = data?.MediaContainer?.Metadata ?? [];
  return items.map(mapItem);
}

export const onDeck = (auth: ServerAuth) =>
  getOrFetch(
    `onDeck:${ku(auth)}`,
    () => fetchList(auth, `/library/onDeck?dummy=1${SLIM_LIST}`),
    { ttlMs: TTL_USER_STATE },
  );

export const recentlyAdded = (auth: ServerAuth) =>
  getOrFetch(
    `recentlyAdded:${ku(auth)}`,
    () =>
      fetchList(
        auth,
        `/library/recentlyAdded?X-Plex-Container-Size=24${SLIM_LIST}`,
      ),
    { ttlMs: TTL_USER_STATE },
  );

export const sectionRecentlyAdded = (auth: ServerAuth, sectionKey: string) =>
  getOrFetch(
    `sectionRecentlyAdded:${k(auth, sectionKey)}`,
    () =>
      fetchList(
        auth,
        `/library/sections/${encodeURIComponent(sectionKey)}/recentlyAdded?X-Plex-Container-Size=24${SLIM_LIST}`,
      ),
    { ttlMs: TTL_SECTION },
  );

export const sectionAll = (
  auth: ServerAuth,
  sectionKey: string,
  limit = 24,
) =>
  getOrFetch(
    `sectionAll:${k(auth, sectionKey, String(limit))}`,
    () =>
      fetchList(
        auth,
        `/library/sections/${encodeURIComponent(sectionKey)}/all?X-Plex-Container-Size=${limit}${SLIM_LIST}`,
      ),
    { ttlMs: TTL_SECTION },
  );

export const sectionTopWatched = (
  auth: ServerAuth,
  sectionKey: string,
  limit = 10,
) =>
  getOrFetch(
    `sectionTopWatched:${k(auth, sectionKey, String(limit))}`,
    () =>
      fetchList(
        auth,
        `/library/sections/${encodeURIComponent(sectionKey)}/all?sort=viewCount:desc&X-Plex-Container-Size=${limit}${SLIM_LIST}`,
      ),
    { ttlMs: TTL_SECTION },
  );

export const sectionTopRated = (
  auth: ServerAuth,
  sectionKey: string,
  limit = 10,
) =>
  getOrFetch(
    `sectionTopRated:${k(auth, sectionKey, String(limit))}`,
    () =>
      fetchList(
        auth,
        `/library/sections/${encodeURIComponent(sectionKey)}/all?sort=rating:desc&X-Plex-Container-Size=${limit}${SLIM_LIST}`,
      ),
    { ttlMs: TTL_SECTION },
  );

export const sectionByGenre = (
  auth: ServerAuth,
  sectionKey: string,
  genre: string,
  limit = 20,
) =>
  getOrFetch(
    `sectionByGenre:${k(auth, sectionKey, genre, String(limit))}`,
    () =>
      fetchList(
        auth,
        `/library/sections/${encodeURIComponent(sectionKey)}/all?genre=${encodeURIComponent(genre)}&X-Plex-Container-Size=${limit}${SLIM_LIST}`,
      ),
    { ttlMs: TTL_SECTION },
  );

export async function getMetadata(
  auth: ServerAuth,
  ratingKey: string,
): Promise<MediaItem | null> {
  return getOrFetch(
    `metadata:${ku(auth, ratingKey)}`,
    async () => {
      // includeMarkers=1 surfaces Plex Pass intro/credits markers (no-op
      // on non-Pass servers, just returns nothing in the Marker array).
      const res = await plexServer(
        `/library/metadata/${encodeURIComponent(ratingKey)}?includeMarkers=1`,
        auth,
      );
      if (!res.ok) return null;
      const data = await res.json();
      const node: MetadataNode | undefined =
        data?.MediaContainer?.Metadata?.[0];
      return node ? mapItem(node) : null;
    },
    { ttlMs: TTL_SECTION },
  );
}

export async function getChildren(
  auth: ServerAuth,
  ratingKey: string,
): Promise<MediaItem[]> {
  return getOrFetch(
    `children:${ku(auth, ratingKey)}`,
    () =>
      fetchList(
        auth,
        `/library/metadata/${encodeURIComponent(ratingKey)}/children?includeMarkers=1`,
      ),
    { ttlMs: TTL_SECTION },
  );
}

export async function getSimilar(
  auth: ServerAuth,
  ratingKey: string,
): Promise<MediaItem[]> {
  return getOrFetch(
    `similar:${k(auth, ratingKey)}`,
    () =>
      fetchList(
        auth,
        `/library/metadata/${encodeURIComponent(ratingKey)}/similar?X-Plex-Container-Size=12`,
      ),
    { ttlMs: TTL_SECTION },
  );
}

/**
 * Plex's /hubs/search returns results bucketed by type ("Movies", "Shows",
 * "Episodes", "Actors", etc). We surface only the media-bearing hubs so
 * the UI can render them with the same Card grid used elsewhere.
 */
export async function searchHubs(
  auth: ServerAuth,
  query: string,
): Promise<import("./plex-types").SearchHub[]> {
  const trimmed = query.trim();
  if (!trimmed) return [];
  return getOrFetch(
    `searchHubs:${k(auth, trimmed.toLowerCase())}`,
    async () => {
      const res = await plexServer(
        `/hubs/search?query=${encodeURIComponent(trimmed)}&limit=24`,
        auth,
      );
      if (!res.ok) return [];
      const data = await res.json();
      const hubs: Array<{
        type?: string;
        hubIdentifier?: string;
        title?: string;
        Metadata?: MetadataNode[];
      }> = data?.MediaContainer?.Hub ?? [];

      // Title-level only: drop the per-episode hub. With long-running
      // shows the episodes hub easily fills the page with dozens of
      // identical-looking thumbs and buries the actual show result.
      const wantedTypes = new Set(["movie", "show"]);
      return hubs
        .filter(
          (h) =>
            wantedTypes.has(String(h.type ?? "")) &&
            Array.isArray(h.Metadata) &&
            h.Metadata.length > 0,
        )
        .map((h) => ({
          type: String(h.type ?? ""),
          title: String(h.title ?? ""),
          items: (h.Metadata ?? []).map(mapItem),
        }));
    },
    { ttlMs: TTL_USER_STATE },
  );
}

export async function sections(auth: ServerAuth): Promise<Section[]> {
  return getOrFetch(
    `sections:${k(auth)}`,
    async () => {
      const res = await plexServer("/library/sections", auth);
      if (!res.ok) return [];
      const data = await res.json();
      const dirs: Array<{ key?: string; title?: string; type?: string }> =
        data?.MediaContainer?.Directory ?? [];
      return dirs.map((d) => ({
        key: String(d.key ?? ""),
        title: String(d.title ?? ""),
        type: String(d.type ?? ""),
      }));
    },
    { ttlMs: TTL_LIBRARIES },
  );
}
