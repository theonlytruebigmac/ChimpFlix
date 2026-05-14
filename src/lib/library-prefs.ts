import { cookies } from "next/headers";
import { isSecureRequest, readActiveServer } from "./session";

const COOKIE = "cf_hidden_libs";

// Cookie value shape: { [serverId]: string[] } — one slice per Plex server
// the user has selected. Plex section keys aren't unique across servers
// (server A's "1" is unrelated to server B's "1"), so a global hidden set
// would falsely hide the wrong libraries after a server switch.
type HiddenMap = Record<string, string[]>;

// Legacy single-slice format. We accept it on read so existing single-
// server sessions don't lose their hidden-library prefs through the
// upgrade — the legacy values get attributed to whatever server is active
// when first read, then migrated to the per-server shape on the next write.
type LegacyHidden = string[];

function parse(raw: string | undefined): HiddenMap | LegacyHidden | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (Array.isArray(parsed)) {
      return parsed.filter((x): x is string => typeof x === "string");
    }
    if (parsed && typeof parsed === "object") {
      const out: HiddenMap = {};
      for (const [k, v] of Object.entries(parsed as Record<string, unknown>)) {
        if (Array.isArray(v)) {
          out[k] = v.filter((x): x is string => typeof x === "string");
        }
      }
      return out;
    }
  } catch {
    // fallthrough to null
  }
  return null;
}

/**
 * Server-side read of the user's hidden-library set, scoped to the
 * currently-selected Plex server. Returns an empty set when no server is
 * selected (e.g. on /select-server before the user has picked one).
 */
export async function readHiddenLibraries(): Promise<Set<string>> {
  const store = await cookies();
  const parsed = parse(store.get(COOKIE)?.value);
  if (!parsed) return new Set();

  const active = await readActiveServer();
  if (!active) return new Set();

  if (Array.isArray(parsed)) {
    // Legacy slice — attribute to the active server.
    return new Set(parsed);
  }
  return new Set(parsed[active.id] ?? []);
}

/**
 * Filters a list of items to only those NOT in a hidden library.
 */
export function filterHiddenItems<
  T extends { librarySectionID?: string | number },
>(items: T[], hidden: Set<string>): T[] {
  if (hidden.size === 0) return items;
  return items.filter(
    (it) =>
      it.librarySectionID === undefined ||
      !hidden.has(String(it.librarySectionID)),
  );
}

/**
 * Writes the hidden-library set for the active server, preserving the
 * stored slices for any other servers. No-op if there's no active server.
 */
export async function writeHiddenLibraries(keys: string[]): Promise<void> {
  const active = await readActiveServer();
  if (!active) return;

  const store = await cookies();
  const current = parse(store.get(COOKIE)?.value);

  const next: HiddenMap = Array.isArray(current)
    ? // Migrate legacy single-slice into the per-server shape; the
      // legacy entries are credited to the active server.
      { [active.id]: current }
    : { ...(current ?? {}) };
  next[active.id] = keys;

  store.set(COOKIE, JSON.stringify(next), {
    httpOnly: false, // readable by client too — non-sensitive UI pref
    sameSite: "lax",
    secure: await isSecureRequest(),
    path: "/",
    maxAge: 60 * 60 * 24 * 365,
  });
}
