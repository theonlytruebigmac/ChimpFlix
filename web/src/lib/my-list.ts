"use client";

import { useCallback, useEffect, useState } from "react";

// My List is now persisted on the server. We keep a client-side Set so
// the toggle button updates instantly while the network call is in flight.
// Legacy localStorage entries (Plex ratingKeys) are migrated on first load:
// numeric ones get POSTed to /api/v1/my-list/<id> and the storage key is
// dropped. Anything non-numeric is dropped — those were Plex-only ids that
// have no Rust equivalent.

const LEGACY_KEY = "cf_mylist";
const EVENT = "app:mylist:changed";

let cache: Set<string> | null = null;
let inflight: Promise<void> | null = null;

function notify(): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new Event(EVENT));
}

async function fetchList(): Promise<Set<string>> {
  const res = await fetch("/api/v1/my-list", { cache: "no-store" });
  if (!res.ok) {
    if (res.status === 401) return new Set();
    throw new Error(`/my-list: ${res.status}`);
  }
  const data = (await res.json()) as { items: { id: number }[] };
  return new Set(data.items.map((i) => String(i.id)));
}

async function migrateLegacy(): Promise<void> {
  if (typeof window === "undefined") return;
  const raw = window.localStorage.getItem(LEGACY_KEY);
  if (!raw) return;
  try {
    const parsed: unknown = JSON.parse(raw);
    const numericIds = new Set<number>();
    const collect = (v: unknown) => {
      if (typeof v === "string") {
        const n = Number.parseInt(v, 10);
        if (Number.isFinite(n) && n > 0) numericIds.add(n);
      } else if (Array.isArray(v)) {
        v.forEach(collect);
      } else if (v && typeof v === "object") {
        Object.values(v as Record<string, unknown>).forEach(collect);
      }
    };
    collect(parsed);
    await Promise.allSettled(
      [...numericIds].map((id) =>
        fetch(`/api/v1/my-list/${id}`, { method: "POST" }),
      ),
    );
  } catch {
    // ignore parse errors
  } finally {
    window.localStorage.removeItem(LEGACY_KEY);
  }
}

async function ensureLoaded(): Promise<void> {
  if (cache) return;
  if (inflight) return inflight;
  inflight = (async () => {
    await migrateLegacy();
    try {
      cache = await fetchList();
    } catch {
      cache = new Set();
    }
    notify();
  })();
  return inflight;
}

export function getMyList(): string[] {
  return cache ? [...cache] : [];
}

export function isInMyList(ratingKey: string): boolean {
  return cache?.has(ratingKey) ?? false;
}

export function addToMyList(ratingKey: string): void {
  const id = Number.parseInt(ratingKey, 10);
  if (!Number.isFinite(id) || id <= 0) return;
  if (!cache) cache = new Set();
  if (cache.has(ratingKey)) return;
  cache.add(ratingKey);
  notify();
  fetch(`/api/v1/my-list/${id}`, { method: "POST" }).catch(() => {
    cache?.delete(ratingKey);
    notify();
  });
}

export function removeFromMyList(ratingKey: string): void {
  const id = Number.parseInt(ratingKey, 10);
  if (!Number.isFinite(id) || id <= 0) {
    // Legacy non-numeric entry — just drop locally.
    cache?.delete(ratingKey);
    notify();
    return;
  }
  if (!cache || !cache.has(ratingKey)) return;
  cache.delete(ratingKey);
  notify();
  fetch(`/api/v1/my-list/${id}`, { method: "DELETE" }).catch(() => {
    cache?.add(ratingKey);
    notify();
  });
}

export const MY_LIST_EVENT = EVENT;

/**
 * Reactive boolean + toggle for a single ratingKey. Subscribes to our
 * custom event so multiple components showing the same item stay in
 * sync within the tab.
 */
export function useMyListItem(ratingKey: string): {
  inList: boolean;
  toggle: () => void;
} {
  const [inList, setInList] = useState(false);

  useEffect(() => {
    function update() {
      setInList(isInMyList(ratingKey));
    }
    void ensureLoaded().then(update);
    window.addEventListener(EVENT, update);
    return () => {
      window.removeEventListener(EVENT, update);
    };
  }, [ratingKey]);

  const toggle = useCallback(() => {
    if (isInMyList(ratingKey)) removeFromMyList(ratingKey);
    else addToMyList(ratingKey);
  }, [ratingKey]);

  return { inList, toggle };
}
