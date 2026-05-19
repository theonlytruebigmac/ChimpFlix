"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { Card } from "./Card";
import { MY_LIST_EVENT } from "@/lib/my-list";
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
  if (!res.ok) return [];
  const data = (await res.json()) as { items: ApiItem[] };
  return data.items.map(adaptItem);
}

export function MyListClient() {
  const [items, setItems] = useState<MediaItem[] | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function load() {
      const result = await fetchMyList();
      if (!cancelled) setItems(result);
    }

    load();
    // Reload whenever any tab/component toggles a saved title so the list
    // reflects the change without a manual refresh.
    window.addEventListener(MY_LIST_EVENT, load);
    return () => {
      cancelled = true;
      window.removeEventListener(MY_LIST_EVENT, load);
    };
  }, []);

  return (
    <div className="px-4 sm:px-8 md:px-12 pb-24 pt-24 sm:pt-28">
      <h1 className="mb-10 text-4xl font-bold tracking-tight">My List</h1>

      {items === null ? (
        <p className="text-white/60">Loading…</p>
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
        <ul className="flex flex-wrap gap-3">
          {items.map((item) => (
            <li key={item.ratingKey} className="flex-none">
              <Card item={item} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
