"use client";

const COOKIE = "cf_hidden_libs";
const SERVER_ID_COOKIE = "cf_server_id";

function readCookie(name: string): string | null {
  if (typeof document === "undefined") return null;
  const match = document.cookie.match(
    new RegExp(`(?:^|;\\s*)${name}=([^;]+)`),
  );
  if (!match) return null;
  let raw = match[1];
  try {
    raw = decodeURIComponent(raw);
  } catch {
    // already decoded
  }
  return raw;
}

/**
 * Client-side reader for the hidden-library set, scoped to the currently-
 * selected Plex server (whose ID we read from the public `cf_server_id`
 * cookie). Server keys aren't unique across servers, so scoping is a
 * correctness requirement, not just a nicety.
 */
export function readHiddenLibrariesClient(): Set<string> {
  const raw = readCookie(COOKIE);
  if (!raw) return new Set();
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return new Set();
  }
  // Legacy format: a flat array. Treat as the active slice when there's
  // no server cookie, or attribute to the current server otherwise.
  if (Array.isArray(parsed)) {
    return new Set(
      parsed.filter((x): x is string => typeof x === "string"),
    );
  }
  if (!parsed || typeof parsed !== "object") return new Set();
  const serverId = readCookie(SERVER_ID_COOKIE);
  if (!serverId) return new Set();
  const slice = (parsed as Record<string, unknown>)[serverId];
  if (!Array.isArray(slice)) return new Set();
  return new Set(slice.filter((x): x is string => typeof x === "string"));
}

export function isHiddenClient(
  librarySectionID: string | number | undefined,
  hidden: Set<string>,
): boolean {
  if (librarySectionID === undefined) return false;
  return hidden.has(String(librarySectionID));
}
