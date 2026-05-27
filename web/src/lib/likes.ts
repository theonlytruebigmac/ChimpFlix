"use client";

import { useCallback, useEffect, useState } from "react";
import { ratings as ratingsApi } from "@/lib/chimpflix-api";
import { devError } from "@/lib/dev-log";

/// "Like" on the Card hover panel reuses the per-user rating endpoints.
/// A like maps to a rating of 8 — Plex's thumbs-up convention — so the
/// detailed RatingBar in TitleModalClient (1–10 chips) stays in sync
/// with the binary thumb in the rail card. `liked` is true for any
/// stored rating, not just 8, so clicking a like in the rail won't
/// accidentally clobber a precise rating the user set in the modal.
export const LIKE_RATING_VALUE = 8;

const RATINGS_EVENT = "app:ratings:changed";

interface RatingChangePayload {
  itemId: number;
  rating: number | null;
}

/// Module-level cache of every item rating the current user has set.
/// Populated lazily on first `useItemLike` mount and kept in sync via
/// the `RATINGS_EVENT` broadcasts emitted by the modal RatingBar and
/// the Card Like button. One `GET /ratings` per session beats N
/// parallel `GET /items/:id/rating` per home-page card load — the
/// per-card fan-out was tripping the global rate limiter (HTTP 429
/// cascade) once the home rails grew past ~20 visible items.
let cache: Map<number, number> | null = null;
let inflight: Promise<void> | null = null;

async function ensureLoaded(): Promise<void> {
  if (cache) return;
  if (inflight) return inflight;
  inflight = (async () => {
    try {
      const all = await ratingsApi.listMine();
      const next = new Map<number, number>();
      for (const [k, v] of Object.entries(all.items)) {
        const id = Number.parseInt(k, 10);
        if (Number.isFinite(id) && id > 0) next.set(id, v);
      }
      cache = next;
    } catch {
      // Network / auth failure — fall back to an empty cache so the UI
      // renders unrated rather than spinning forever. Subsequent
      // mutations will still hit the per-id endpoints and repopulate.
      cache = new Map();
    }
  })();
  return inflight;
}

/// Broadcast a rating change so Card likes + the modal RatingBar stay
/// aligned within the tab. Call after the API write succeeds.
export function notifyRatingChanged(itemId: number, rating: number | null) {
  if (typeof window === "undefined") return;
  // Keep the module-level cache hot so any Card that mounts after the
  // change reflects it without a refetch.
  if (cache) {
    if (rating === null) cache.delete(itemId);
    else cache.set(itemId, rating);
  }
  window.dispatchEvent(
    new CustomEvent<RatingChangePayload>(RATINGS_EVENT, {
      detail: { itemId, rating },
    }),
  );
}

/// Reactive boolean + toggle for a single item's Like state, backed by
/// the per-user rating API. `ratingKey` is the Card modal key — for
/// items this is the numeric id as a string. Episodes don't appear in
/// rails so we don't need to handle the `e<id>` form here; if a non-
/// numeric key sneaks in, `liked` stays false and `toggle` is a no-op.
export function useItemLike(ratingKey: string): {
  liked: boolean;
  toggle: () => void;
  loading: boolean;
} {
  const itemId = Number.parseInt(ratingKey, 10);
  const valid = Number.isFinite(itemId) && itemId > 0;
  const [rating, setRating] = useState<number | null>(null);
  const [loading, setLoading] = useState(valid);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!valid) return;
    let cancelled = false;
    void ensureLoaded().then(() => {
      if (cancelled) return;
      setRating(cache?.get(itemId) ?? null);
      setLoading(false);
    });

    function onChange(e: Event) {
      const detail = (e as CustomEvent<RatingChangePayload>).detail;
      if (detail.itemId === itemId) setRating(detail.rating);
    }
    window.addEventListener(RATINGS_EVENT, onChange);
    return () => {
      cancelled = true;
      window.removeEventListener(RATINGS_EVENT, onChange);
    };
  }, [itemId, valid]);

  const toggle = useCallback(() => {
    if (!valid || busy) return;
    const wasLiked = rating !== null;
    // Optimistic — flip immediately so the icon doesn't lag the click.
    const optimistic = wasLiked ? null : LIKE_RATING_VALUE;
    setRating(optimistic);
    setBusy(true);
    (async () => {
      try {
        if (wasLiked) {
          await ratingsApi.deleteItem(itemId);
          notifyRatingChanged(itemId, null);
        } else {
          const r = await ratingsApi.putItem(itemId, LIKE_RATING_VALUE);
          const final = r.rating ?? LIKE_RATING_VALUE;
          setRating(final);
          notifyRatingChanged(itemId, final);
        }
      } catch (e) {
        // Revert the optimistic toggle. We don't have a global toast
        // system to surface this — log to console so dev tools shows
        // the network failure, since the silent flip-back used to leave
        // users confused ("why did my like un-like itself?").
        devError("[likes] rating PUT/DELETE failed:", e);
        setRating(wasLiked ? rating : null);
      } finally {
        setBusy(false);
      }
    })();
  }, [busy, itemId, rating, valid]);

  return { liked: rating !== null, toggle, loading };
}
