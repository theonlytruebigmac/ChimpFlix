"use client";

import { useCallback, useEffect, useState } from "react";

const KEY = "cf_mylist";
const SERVER_ID_COOKIE = "cf_server_id";
const EVENT = "app:mylist:changed";

// localStorage value shape: { [serverId]: string[] } — Plex ratingKeys are
// only unique within a server, so scoping My List per-server prevents
// ghost entries after a server switch (and lets a user keep separate
// lists for each server they have access to).
type MyListMap = Record<string, string[]>;

function readServerId(): string | null {
  if (typeof document === "undefined") return null;
  const m = document.cookie.match(
    new RegExp(`(?:^|;\\s*)${SERVER_ID_COOKIE}=([^;]+)`),
  );
  if (!m) return null;
  try {
    return decodeURIComponent(m[1]);
  } catch {
    return m[1];
  }
}

function readAll(): MyListMap {
  if (typeof window === "undefined") return {};
  try {
    const raw = window.localStorage.getItem(KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (Array.isArray(parsed)) {
      // Legacy flat-array shape from before multi-tenant. Attribute it
      // to whatever server is currently active so users don't lose
      // their list through the upgrade.
      const list = parsed.filter((x): x is string => typeof x === "string");
      const serverId = readServerId();
      return serverId ? { [serverId]: list } : {};
    }
    if (!parsed || typeof parsed !== "object") return {};
    const out: MyListMap = {};
    for (const [k, v] of Object.entries(parsed as Record<string, unknown>)) {
      if (Array.isArray(v)) {
        out[k] = v.filter((x): x is string => typeof x === "string");
      }
    }
    return out;
  } catch {
    return {};
  }
}

function writeAll(map: MyListMap): void {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(KEY, JSON.stringify(map));
  window.dispatchEvent(new Event(EVENT));
}

function read(): string[] {
  const serverId = readServerId();
  if (!serverId) return [];
  return readAll()[serverId] ?? [];
}

function write(list: string[]): void {
  const serverId = readServerId();
  if (!serverId) return;
  const all = readAll();
  all[serverId] = list;
  writeAll(all);
}

export function getMyList(): string[] {
  return read();
}

export function isInMyList(ratingKey: string): boolean {
  return read().includes(ratingKey);
}

export function addToMyList(ratingKey: string): void {
  const list = read();
  if (!list.includes(ratingKey)) {
    write([ratingKey, ...list]);
  }
}

export function removeFromMyList(ratingKey: string): void {
  const list = read();
  const next = list.filter((k) => k !== ratingKey);
  if (next.length !== list.length) write(next);
}

export const MY_LIST_EVENT = EVENT;

/**
 * Reactive boolean + toggle for a single ratingKey. Subscribes to our
 * custom event so multiple components showing the same item stay in
 * sync within the tab; the native `storage` event also covers cross-
 * tab updates.
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
    update();
    window.addEventListener(EVENT, update);
    window.addEventListener("storage", update);
    return () => {
      window.removeEventListener(EVENT, update);
      window.removeEventListener("storage", update);
    };
  }, [ratingKey]);

  const toggle = useCallback(() => {
    if (isInMyList(ratingKey)) removeFromMyList(ratingKey);
    else addToMyList(ratingKey);
  }, [ratingKey]);

  return { inList, toggle };
}
