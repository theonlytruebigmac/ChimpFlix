"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { Card } from "./Card";
import { CardSkeleton } from "./Skeleton";
import { MY_LIST_EVENT, getMyList } from "@/lib/my-list";
import { adaptItem } from "@/lib/chimpflix-adapt";
import type { MediaItem } from "@/lib/chimpflix-types";

interface ApiItem {
  id: number;
  library_id: number;
  kind: "movie" | "show";
  title: string;
  sort_title: string;
  original_title: string | null;
  year: number | null;
  summary: string | null;
  tagline: string | null;
  duration_ms: number | null;
  rating_audience: number | null;
  tmdb_id: number | null;
  imdb_id: string | null;
  tvdb_id: number | null;
  anilist_id: number | null;
  poster_path: string | null;
  backdrop_path: string | null;
  logo_path: string | null;
  added_at: number;
  updated_at: number;
  play_state: {
    position_ms: number;
    duration_ms: number | null;
    watched: boolean;
    view_count: number;
    last_played_at: number;
  } | null;
}

async function fetchMyList(): Promise<MediaItem[]> {
  const res = await fetch("/api/v1/my-list", { cache: "no-store" });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const data = (await res.json()) as { items: ApiItem[] };
  return data.items.map(adaptItem);
}

export function MyListClient() {
  const [items, setItems] = useState<MediaItem[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        const result = await fetchMyList();
        if (!cancelled) {
          setItems(result);
          setError(null);
        }
      } catch (e) {
        if (!cancelled) {
          // Distinguish a real fetch failure from an empty list. The
          // old fall-through-to-[] path was indistinguishable from the
          // empty state and left users wondering why their saved list
          // disappeared after a network blip.
          setError(e instanceof Error ? e.message : String(e));
          setItems([]);
        }
      }
    }

    load();

    // On MY_LIST_EVENT, sync from the client-side cache instead of
    // re-fetching from the server. The event fires before the POST/DELETE
    // response completes, so a server re-fetch would race and could return
    // stale data. Reading getMyList() (the optimistic Set) is consistent with
    // the state the toggle already applied. A server re-fetch (load()) is
    // still used on failure rollback because the cache reverts and fires
    // another event, and on mount for the initial full item list.
    function syncFromCache() {
      if (cancelled) return;
      const ids = new Set(getMyList());
      setItems((prev) =>
        prev === null ? prev : prev.filter((it) => ids.has(it.ratingKey)),
      );
    }

    window.addEventListener(MY_LIST_EVENT, syncFromCache);
    return () => {
      cancelled = true;
      window.removeEventListener(MY_LIST_EVENT, syncFromCache);
    };
  }, []);

  return (
    <div className="px-4 sm:px-8 md:px-12 pb-24 pt-24 sm:pt-28">
      <h1 className="mb-10 text-4xl font-bold tracking-tight">My List</h1>

      {error && (
        <div
          role="alert"
          aria-live="assertive"
          className="mb-6 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-300"
        >
          Couldn&apos;t load your list: {error}. Try refreshing.
        </div>
      )}

      {items === null ? (
        // Skeleton grid mirroring the loaded layout — every other browse
        // surface uses CardSkeleton, so the bare "Loading…" used to read
        // as broken on a slow first paint. 12 cards is enough to cover
        // the typical above-the-fold viewport without flashing too much
        // chrome on a small list.
        <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
          {Array.from({ length: 12 }).map((_, i) => (
            <li key={i}>
              <CardSkeleton />
            </li>
          ))}
        </ul>
      ) : items.length === 0 ? (
        <div className="max-w-xl">
          <p className="mb-3 text-base text-white/85">Your list is empty.</p>
          <p className="text-sm text-white/60">
            Hover over any title and tap the{" "}
            <span className="inline-flex h-5 w-5 items-center justify-center rounded-full border border-white/50 align-middle text-xs">
              +
            </span>{" "}
            button to save it here for later.
          </p>
          <Link
            href="/"
            className="mt-6 inline-block text-sm text-white underline underline-offset-4 hover:text-(--color-accent)"
          >
            Browse titles
          </Link>
        </div>
      ) : (
        // Match the grid layout used everywhere else (/search,
        // /library/[id]/browse, /genre, /collection, /history). The
        // previous flex/wrap layout left cards at their natural width,
        // which produced ragged right edges and didn't line up with
        // the other discovery surfaces.
        <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
          {items.map((item) => (
            <li key={item.ratingKey}>
              <Card item={item} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
