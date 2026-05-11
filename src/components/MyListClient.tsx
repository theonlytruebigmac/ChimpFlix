"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { Card } from "./Card";
import {
  getMyList,
  MY_LIST_EVENT,
  removeFromMyList,
} from "@/lib/my-list";
import {
  isHiddenClient,
  readHiddenLibrariesClient,
} from "@/lib/library-prefs-client";
import {
  mapItem,
  type MediaItem,
  type MetadataNode,
} from "@/lib/plex-types";

async function fetchItem(ratingKey: string): Promise<MediaItem | null> {
  try {
    const res = await fetch(
      `/api/plex/library/metadata/${encodeURIComponent(ratingKey)}`,
    );
    if (!res.ok) {
      // 404 — item no longer exists in Plex; clean it out.
      if (res.status === 404) removeFromMyList(ratingKey);
      return null;
    }
    const data = await res.json();
    const node: MetadataNode | undefined =
      data?.MediaContainer?.Metadata?.[0];
    return node ? mapItem(node) : null;
  } catch {
    return null;
  }
}

export function MyListClient() {
  const [items, setItems] = useState<MediaItem[] | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function load() {
      const keys = getMyList();
      if (keys.length === 0) {
        if (!cancelled) setItems([]);
        return;
      }
      const results = await Promise.all(keys.map(fetchItem));
      if (cancelled) return;
      const hidden = readHiddenLibrariesClient();
      setItems(
        results
          .filter((x): x is MediaItem => x !== null)
          .filter((x) => !isHiddenClient(x.librarySectionID, hidden)),
      );
    }

    load();
    window.addEventListener(MY_LIST_EVENT, load);
    return () => {
      cancelled = true;
      window.removeEventListener(MY_LIST_EVENT, load);
    };
  }, []);

  return (
    <div className="px-12 pb-24 pt-28">
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
