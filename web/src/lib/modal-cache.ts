// Client-side cache + prefetcher for modal data. Card.tsx calls
// `prefetchModalData(ratingKey)` on hover; by the time the user clicks (or
// even hovers long enough for the hover-panel to expand and they click the
// More Info arrow), the underlying server fetch is already in flight or
// resolved. TitleModalClient looks up the same cache, so opening the modal
// becomes a no-op render against an already-resolved promise.
//
// The actual data assembly (metadata + similar + children + first season's
// episodes + hidden-library filter) happens server-side at /api/modal/...,
// where it can ride the in-memory cache shared with page renders. That
// turns a 4-roundtrip cold modal open into a 1-roundtrip warm one.

import { type MediaItem } from "./chimpflix-types";
import type { Credit, Extra, ItemDetail, ReviewsSummary } from "./chimpflix-api";

export type ModalData = {
  item: MediaItem;
  seasons: MediaItem[];
  initialEpisodes: MediaItem[];
  similar: MediaItem[];
  credits: Credit[];
  extras: Extra[];
  reviews: ReviewsSummary;
  locked_fields: string[];
  /// Raw backend ItemDetail. The visible UI uses the adapted `item` field;
  /// Edit / Fix Match dialogs use this for the original numeric id, tmdb_id,
  /// rating_audience, etc.
  detail: ItemDetail;
};

const cache = new Map<string, Promise<ModalData | null>>();

async function loadModalData(
  ratingKey: string,
): Promise<ModalData | null> {
  try {
    const res = await fetch(
      `/api/modal/${encodeURIComponent(ratingKey)}`,
    );
    if (res.status === 404) return null;
    if (!res.ok) {
      throw new Error(`modal: ${res.status}`);
    }
    return (await res.json()) as ModalData;
  } catch (e) {
    // Allow retry on next call.
    cache.delete(ratingKey);
    throw e;
  }
}

export function prefetchModalData(ratingKey: string): void {
  if (typeof window === "undefined") return;
  if (cache.has(ratingKey)) return;
  const p = loadModalData(ratingKey);
  cache.set(ratingKey, p);
  // Swallow the rejection here so a failed prefetch doesn't surface as an
  // unhandled rejection. The actual click site awaits the same promise via
  // getOrFetchModalData and handles the error.
  p.catch(() => {});
}

export function getOrFetchModalData(
  ratingKey: string,
): Promise<ModalData | null> {
  let p = cache.get(ratingKey);
  if (!p) {
    p = loadModalData(ratingKey);
    cache.set(ratingKey, p);
  }
  return p;
}
