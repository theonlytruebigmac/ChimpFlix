import { cookies, headers } from "next/headers";
import { createHmac, timingSafeEqual } from "node:crypto";
import { env } from "./env";

// Decide whether to set the Secure flag on cookies.
//
//   - If APP_PUBLIC_ORIGIN is set, it is authoritative: secure iff the
//     configured origin is https. This is the right answer in any
//     production deployment because it doesn't trust attacker-supplied
//     request headers.
//   - If APP_TRUST_PROXY=1, honor X-Forwarded-Proto from the proxy.
//   - Otherwise default to *not* setting Secure. This is correct for a
//     plain-HTTP LAN deployment and prevents a MITM on the LAN from
//     downgrading by spoofing X-Forwarded-Proto: http.
export async function isSecureRequest(): Promise<boolean> {
  const configured = env.publicOrigin();
  if (configured) return configured.startsWith("https://");
  if (!env.trustProxy()) return false;
  try {
    const h = await headers();
    const xfp = h.get("x-forwarded-proto");
    return xfp?.split(",")[0]?.trim() === "https";
  } catch {
    return false;
  }
}

const TOKEN_COOKIE = "cf_token"; // currently-active profile
const ADMIN_COOKIE = "cf_admin"; // original Plex account (for switching)
const PENDING_COOKIE = "cf_pending";
// Cached identity (title/thumb/etc.) for the active profile. Plex's
// /api/v2/user endpoint only works for the master account's token; managed
// home-user tokens get 401 there, which means we can't look up an avatar
// directly. Stashing it at switch time gives us a reliable source for the
// nav avatar regardless of which profile is active.
const PROFILE_COOKIE = "cf_profile";
// Currently-selected Plex server. With multi-tenant we no longer pin every
// session to one configured PLEX_SERVER_URL — each user picks from the
// servers their plex.tv account has access to, and we store id/url/token
// here. The accessToken is server-scoped (different from cf_token) and is
// what gets sent on every Plex Media Server API call.
const SERVER_COOKIE = "cf_server";
// Mirror of just the server ID, readable by client-side code (the main
// server cookie is httpOnly because it carries an access token). Lets the
// client scope localStorage / preferences cookies by server without
// needing to round-trip to /api/auth/me on every read.
const SERVER_ID_COOKIE = "cf_server_id";

function sign(value: string): string {
  return createHmac("sha256", env.sessionSecret()).update(value).digest("hex");
}

function pack(value: string): string {
  return `${value}.${sign(value)}`;
}

function unpack(packed: string | undefined): string | null {
  if (!packed) return null;
  const idx = packed.lastIndexOf(".");
  if (idx < 0) return null;
  const value = packed.slice(0, idx);
  const sig = packed.slice(idx + 1);
  const expected = sign(value);
  const a = Buffer.from(sig, "hex");
  const b = Buffer.from(expected, "hex");
  if (a.length !== b.length || !timingSafeEqual(a, b)) return null;
  return value;
}

export async function readToken(): Promise<string | null> {
  const store = await cookies();
  return unpack(store.get(TOKEN_COOKIE)?.value);
}

export async function writeToken(token: string): Promise<void> {
  const store = await cookies();
  store.set(TOKEN_COOKIE, pack(token), {
    httpOnly: true,
    sameSite: "lax",
    secure: await isSecureRequest(),
    path: "/",
    maxAge: 60 * 60 * 24 * 365,
  });
}

export async function clearToken(): Promise<void> {
  const store = await cookies();
  store.delete(TOKEN_COOKIE);
}

export async function readAdminToken(): Promise<string | null> {
  const store = await cookies();
  const admin = unpack(store.get(ADMIN_COOKIE)?.value);
  if (admin) return admin;
  // Backward-compat: sessions created before the dual-cookie roll-out only
  // have cf_token. Treat that as the admin token too — the first sign-in's
  // token is naturally both admin and active.
  return unpack(store.get(TOKEN_COOKIE)?.value);
}

export async function writeAdminToken(token: string): Promise<void> {
  const store = await cookies();
  store.set(ADMIN_COOKIE, pack(token), {
    httpOnly: true,
    sameSite: "lax",
    secure: await isSecureRequest(),
    path: "/",
    maxAge: 60 * 60 * 24 * 365,
  });
}

export async function clearAdminToken(): Promise<void> {
  const store = await cookies();
  store.delete(ADMIN_COOKIE);
}

export type ActiveProfile = {
  id: number;
  username: string;
  email: string;
  thumb: string;
};

export async function readActiveProfile(): Promise<ActiveProfile | null> {
  const store = await cookies();
  const raw = unpack(store.get(PROFILE_COOKIE)?.value);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as ActiveProfile;
    if (typeof parsed?.username !== "string") return null;
    return parsed;
  } catch {
    return null;
  }
}

export async function writeActiveProfile(
  profile: ActiveProfile,
): Promise<void> {
  const store = await cookies();
  store.set(PROFILE_COOKIE, pack(JSON.stringify(profile)), {
    httpOnly: true,
    sameSite: "lax",
    secure: await isSecureRequest(),
    path: "/",
    maxAge: 60 * 60 * 24 * 365,
  });
}

export async function clearActiveProfile(): Promise<void> {
  const store = await cookies();
  store.delete(PROFILE_COOKIE);
}

// Re-export the ServerAuth shape so callers can read it directly without
// re-importing from plex.ts.
export type { ServerAuth } from "./plex";

export async function readActiveServer(): Promise<
  import("./plex").ServerAuth | null
> {
  const store = await cookies();
  const raw = unpack(store.get(SERVER_COOKIE)?.value);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as import("./plex").ServerAuth;
    if (!parsed?.url || !parsed?.accessToken || !parsed?.id) return null;
    return parsed;
  } catch {
    return null;
  }
}

export async function writeActiveServer(
  server: import("./plex").ServerAuth,
): Promise<void> {
  const store = await cookies();
  store.set(SERVER_COOKIE, pack(JSON.stringify(server)), {
    httpOnly: true,
    sameSite: "lax",
    secure: await isSecureRequest(),
    path: "/",
    maxAge: 60 * 60 * 24 * 365,
  });
  // Public mirror of the server ID for client-side readers (my list,
  // hidden-library lookup). Carries no auth material so it's safe to be
  // readable by JS.
  store.set(SERVER_ID_COOKIE, server.id, {
    httpOnly: false,
    sameSite: "lax",
    secure: await isSecureRequest(),
    path: "/",
    maxAge: 60 * 60 * 24 * 365,
  });
}

export async function clearActiveServer(): Promise<void> {
  const store = await cookies();
  store.delete(SERVER_COOKIE);
  store.delete(SERVER_ID_COOKIE);
}

/**
 * Resolves the per-request Plex server auth. Prefers the `cf_server`
 * cookie set by the multi-tenant flow. Falls back to the legacy
 * single-server config (`PLEX_SERVER_URL` env + `cf_token` cookie) so
 * existing sessions keep working through an upgrade. Returns null when
 * the session has no token at all.
 */
export async function resolveServerAuth(): Promise<
  import("./plex").ServerAuth | null
> {
  const fromCookie = await readActiveServer();
  if (fromCookie) return fromCookie;

  // Legacy single-server fallback. The active session token doubles as
  // the per-server access token because in single-server mode they were
  // never distinguished.
  const token = await readToken();
  const url = env.plexServerUrlDefault();
  if (!token || !url) return null;
  return {
    id: "default",
    url,
    accessToken: token,
  };
}

/**
 * Canonical entry point for any authenticated page. Returns the active
 * ServerAuth and ensures the cache warmer is running for it. Redirects
 * to /login (no token at all) or /select-server (token but no server
 * picked) when there's nothing to render against.
 *
 * Pages call this in place of readToken() at the top of their async
 * server component:
 *
 *     const auth = await requireServerAuth();
 *
 * From there, every plex-data helper takes `auth` and the warmer is
 * pre-warming whatever the user picks.
 */
export async function requireServerAuth(): Promise<
  import("./plex").ServerAuth
> {
  const { redirect } = await import("next/navigation");
  const auth = await resolveServerAuth();
  if (auth) {
    // Side-effect: bootstrap the warmer. Idempotent on repeat calls
    // with the same auth tuple.
    const { ensureWarmerStarted } = await import("./cache-warmer");
    ensureWarmerStarted(auth);
    return auth;
  }
  const token = await readToken();
  redirect(token ? "/select-server" : "/login");
  // TypeScript can't follow that the dynamically-imported `redirect`
  // returns `never`, so make it explicit. Unreachable at runtime.
  throw new Error("unreachable");
}

export async function readPendingPin(): Promise<number | null> {
  const store = await cookies();
  const raw = unpack(store.get(PENDING_COOKIE)?.value);
  if (!raw) return null;
  const n = Number(raw);
  return Number.isFinite(n) ? n : null;
}

export async function writePendingPin(pinId: number): Promise<void> {
  const store = await cookies();
  store.set(PENDING_COOKIE, pack(String(pinId)), {
    httpOnly: true,
    sameSite: "lax",
    secure: await isSecureRequest(),
    path: "/",
    maxAge: 60 * 30,
  });
}

export async function clearPendingPin(): Promise<void> {
  const store = await cookies();
  store.delete(PENDING_COOKIE);
}
