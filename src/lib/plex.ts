import { env } from "./env";

const PLEX_TV = "https://plex.tv/api/v2";

function plexHeaders(extra: Record<string, string> = {}): HeadersInit {
  return {
    Accept: "application/json",
    "X-Plex-Product": env.plexProductName(),
    "X-Plex-Version": "0.1.0",
    "X-Plex-Client-Identifier": env.plexClientIdentifier(),
    "X-Plex-Platform": "Web",
    "X-Plex-Device": env.plexDeviceName(),
    "X-Plex-Device-Name": env.plexDeviceName(),
    ...extra,
  };
}

// Some Plex API responses (resources, home/users, switch) embed
// authToken / accessToken values in the body. When a request fails and
// we surface the body in an error, those tokens would leak into the
// browser via JSON error responses. Scrub them. Server-side logging
// happens before scrub so operators still see full payloads.
function scrubTokens(s: string): string {
  return (
    s
      // JSON / XML attribute form with double or single quotes. Covers
      // all three token names, including the quoted-JSON form of
      // `"X-Plex-Token":"abc"`.
      .replace(
        /(["']?(?:authToken|accessToken|X-Plex-Token)["']?\s*[:=]\s*)(["'])[^"']*\2/gi,
        '$1$2<redacted>$2',
      )
      // Query-string / form form: authToken=abc
      .replace(/(authToken|accessToken)=[^&\s"'<>]+/gi, "$1=<redacted>")
      // Plain header / URL-param form (no quotes around the value).
      // Keep the original separator so error output stays readable.
      .replace(
        /(X-Plex-Token)(\s*[:=]\s*)[^&\s"'<>,;)\]}]+/gi,
        "$1$2<redacted>",
      )
  );
}

// Wraps a non-OK Plex response into an Error whose message is safe to
// return to the browser. Logs the full unredacted body server-side for
// diagnostics.
async function plexErrorFromResponse(
  res: Response,
  prefix: string,
): Promise<Error> {
  let body = "";
  try {
    body = await res.text();
  } catch {
    // body unavailable — status alone is enough
  }
  console.error(`[plex] ${prefix} failed ${res.status}:`, body);
  const snippet = scrubTokens(body).slice(0, 200);
  return new Error(`${prefix}: ${res.status}${snippet ? ` ${snippet}` : ""}`);
}

export type Pin = {
  id: number;
  code: string;
  authToken: string | null;
  expiresAt: string;
};

export async function createPin(opts: { strong?: boolean } = {}): Promise<Pin> {
  // strong=true mints a long opaque code (24 chars) that the OAuth
  // redirect flow uses — too long for manual entry at plex.tv/link, but
  // perfect for passing through Plex's `?code=` URL param. The fallback
  // (no `strong`) is a 4-char code suitable for type-it-yourself flows.
  const url = opts.strong ? `${PLEX_TV}/pins?strong=true` : `${PLEX_TV}/pins`;
  const res = await fetch(url, {
    method: "POST",
    headers: plexHeaders(),
    cache: "no-store",
  });
  if (!res.ok) throw new Error(`createPin: ${res.status}`);
  const data = await res.json();
  return {
    id: data.id,
    code: data.code,
    authToken: data.authToken ?? null,
    expiresAt: data.expiresAt,
  };
}

export async function checkPin(pinId: number): Promise<Pin> {
  const res = await fetch(`${PLEX_TV}/pins/${pinId}`, {
    headers: plexHeaders(),
    cache: "no-store",
  });
  if (!res.ok) throw new Error(`checkPin: ${res.status}`);
  const data = await res.json();
  return {
    id: data.id,
    code: data.code,
    authToken: data.authToken ?? null,
    expiresAt: data.expiresAt,
  };
}

export function plexLinkUrl(code: string): string {
  return `https://plex.tv/link?code=${encodeURIComponent(code)}`;
}

/**
 * Builds Plex's OAuth-style authorization URL. Redirecting the browser
 * here is the smooth equivalent of "Sign in with Plex": if the user is
 * already signed in at plex.tv (almost always true), it's a one-click
 * authorize and an immediate bounce back to `forwardUrl`. The strong
 * PIN code is included in the URL hash, so the user never sees it.
 *
 * Plex's frontend reads params from the hash (note the `#?`), not the
 * query string — that's a Plex convention, not a typo.
 */
export function plexOAuthUrl(code: string, forwardUrl: string): string {
  const params = new URLSearchParams({
    clientID: env.plexClientIdentifier(),
    code,
    "context[device][product]": env.plexProductName(),
    "context[device][device]": env.plexDeviceName(),
    "context[device][version]": "0.1.0",
    "context[device][platform]": "Web",
    forwardUrl,
  });
  return `https://app.plex.tv/auth#?${params.toString()}`;
}

export type PlexUser = {
  id: number;
  uuid: string;
  username: string;
  email: string;
  thumb: string;
};

export type HomeUser = {
  id: number;
  uuid: string;
  title: string;
  username: string;
  thumb: string;
  hasPassword: boolean;
  protected: boolean;
  restricted: boolean;
  admin: boolean;
};

type RawHomeUser = {
  id?: number;
  uuid?: string;
  title?: string;
  username?: string;
  thumb?: string;
  hasPassword?: boolean;
  protected?: boolean;
  restricted?: boolean;
  admin?: boolean;
};

/**
 * Lists the Plex Home / managed-user profiles available under the admin's
 * account. Returns just the admin themselves on accounts without Plex Home.
 *
 * Cached for 5 minutes via Next's data cache. Plex Home membership changes
 * rarely (when the admin manually adds/removes a user on plex.tv), and the
 * /switch page hits this on every visit; without caching it adds a 200-500ms
 * plex.tv roundtrip each time the user opens the picker.
 */
export async function getHomeUsers(adminToken: string): Promise<HomeUser[]> {
  const res = await fetch(`${PLEX_TV}/home/users`, {
    headers: plexHeaders({ "X-Plex-Token": adminToken }),
    next: { revalidate: 300 },
  });
  if (!res.ok) {
    throw await plexErrorFromResponse(res, "getHomeUsers");
  }
  const data = await res.json();
  // Different Plex API responses wrap the user list in different shapes
  // depending on server version: `{users:[...]}`, `{User:[...]}`, or just a
  // top-level array. Handle each so a quirk doesn't silently drop everyone.
  const users: RawHomeUser[] = Array.isArray(data)
    ? (data as RawHomeUser[])
    : Array.isArray(data?.users)
      ? (data.users as RawHomeUser[])
      : Array.isArray(data?.User)
        ? (data.User as RawHomeUser[])
        : [];
  return users.map((u) => ({
    id: Number(u.id ?? 0),
    uuid: String(u.uuid ?? ""),
    title: String(u.title ?? u.username ?? ""),
    username: String(u.username ?? u.title ?? ""),
    thumb: String(u.thumb ?? ""),
    hasPassword: Boolean(u.hasPassword),
    protected: Boolean(u.protected),
    restricted: Boolean(u.restricted),
    admin: Boolean(u.admin),
  }));
}

/**
 * Exchanges the admin's authToken (plus optional PIN) for a profile-scoped
 * authToken. The new token is what we store as the active session token; the
 * admin token stays alongside so the user can switch back without re-doing
 * the full Plex sign-in.
 *
 * Plex has shipped multiple URL shapes for this over the years (v2 returning
 * JSON, legacy v1 returning XML). We try each in order and parse whichever
 * format comes back so we don't break when Plex flips a flag on their side.
 */
export async function switchHomeUser(
  adminToken: string,
  userId: number,
  pin?: string,
): Promise<string> {
  const pinSuffix = pin ? `?pin=${encodeURIComponent(pin)}` : "";
  const candidates = [
    `https://plex.tv/api/v2/home/users/${userId}/switch${pinSuffix}`,
    `https://plex.tv/api/home/users/${userId}/switch${pinSuffix}`,
  ];

  let lastError = "no candidate URL responded";

  for (const url of candidates) {
    let res: Response;
    try {
      res = await fetch(url, {
        method: "POST",
        headers: plexHeaders({ "X-Plex-Token": adminToken }),
        cache: "no-store",
      });
    } catch (e) {
      lastError = e instanceof Error ? e.message : String(e);
      continue;
    }

    // Wrong PIN should bubble up immediately so the picker can prompt
    // again instead of falling through and confusing the error message.
    if (res.status === 401) {
      throw new Error("switchHomeUser: 401 (wrong PIN)");
    }

    if (!res.ok) {
      const body = await res.text().catch(() => "");
      console.error(`[plex] switchHomeUser ${res.status} from ${url}:`, body);
      lastError = `${res.status} from ${url} — ${scrubTokens(body).slice(0, 200)}`;
      continue;
    }

    const text = await res.text();
    const token = parseAuthToken(text);
    if (token) return token;

    console.error(`[plex] switchHomeUser 200 but no authToken:`, text);
    lastError = `200 but no authToken in body: ${scrubTokens(text).slice(0, 200)}`;
  }

  throw new Error(`switchHomeUser: ${lastError}`);
}

function parseAuthToken(body: string): string | null {
  // Try JSON first (v2 endpoint typical response).
  try {
    const data = JSON.parse(body) as {
      authToken?: string;
      user?: { authToken?: string };
    };
    const token = data.authToken ?? data.user?.authToken;
    if (typeof token === "string" && token) return token;
  } catch {
    // fall through to XML
  }
  // Legacy XML response: <user authToken="..."/>
  const m = body.match(/authToken="([^"]+)"/);
  return m ? m[1] : null;
}

export async function getUser(token: string): Promise<PlexUser> {
  const res = await fetch(`${PLEX_TV}/user`, {
    headers: plexHeaders({ "X-Plex-Token": token }),
    cache: "no-store",
  });
  if (!res.ok) throw new Error(`getUser: ${res.status}`);
  const d = await res.json();
  return {
    id: d.id,
    uuid: d.uuid,
    username: d.username,
    email: d.email,
    thumb: d.thumb,
  };
}

// ── Server discovery (multi-tenant) ──────────────────────────────────────
//
// `/api/v2/resources` returns every Plex server the master token has access
// to: ones the user owns, plus servers shared with them. Each entry carries
// a per-share `accessToken` that's what we want to send on subsequent
// server calls — it's always valid for that server, even when the master
// token isn't (e.g. shared servers may scope what a Friend can see).

export type PlexConnection = {
  protocol: string;
  address: string;
  port: number;
  uri: string;
  local: boolean;
  relay: boolean;
  ipv6: boolean;
};

export type PlexResource = {
  // Stable per-server identifier from Plex; survives URL changes.
  id: string;
  name: string;
  product: string;
  productVersion: string;
  platform: string;
  // Per-server access token. Use this (not the master token) for calls
  // against this server.
  accessToken: string;
  owned: boolean;
  presence: boolean;
  publicAddress: string;
  connections: PlexConnection[];
};

type RawConnection = {
  protocol?: string;
  address?: string;
  port?: number;
  uri?: string;
  local?: boolean;
  relay?: boolean;
  IPv6?: boolean;
  ipv6?: boolean;
};

type RawResource = {
  name?: string;
  product?: string;
  productVersion?: string;
  platform?: string;
  clientIdentifier?: string;
  accessToken?: string;
  provides?: string;
  owned?: boolean;
  presence?: boolean;
  publicAddress?: string;
  connections?: RawConnection[];
};

export async function getResources(
  masterToken: string,
): Promise<PlexResource[]> {
  // Token is sent via the X-Plex-Token header only. Embedding it in the
  // URL is redundant and leaks tokens into anything that logs URLs
  // (DNS resolvers, transparent proxies, error stack traces).
  const res = await fetch(
    `${PLEX_TV}/resources?includeHttps=1&includeRelay=1`,
    {
      headers: plexHeaders({ "X-Plex-Token": masterToken }),
      next: { revalidate: 300 },
    },
  );
  if (!res.ok) {
    throw await plexErrorFromResponse(res, "getResources");
  }
  const data = (await res.json()) as unknown;
  const list: RawResource[] = Array.isArray(data)
    ? (data as RawResource[])
    : ((data as { MediaContainer?: { Device?: RawResource[] } } | null)
        ?.MediaContainer?.Device ?? []);

  return list
    .filter(
      (d) =>
        typeof d.provides === "string" &&
        d.provides.split(",").includes("server") &&
        d.product === "Plex Media Server",
    )
    .map((d) => ({
      id: String(d.clientIdentifier ?? ""),
      name: String(d.name ?? ""),
      product: String(d.product ?? ""),
      productVersion: String(d.productVersion ?? ""),
      platform: String(d.platform ?? ""),
      accessToken: String(d.accessToken ?? masterToken),
      owned: Boolean(d.owned),
      presence: Boolean(d.presence),
      publicAddress: String(d.publicAddress ?? ""),
      connections: (d.connections ?? []).map((c) => ({
        protocol: String(c.protocol ?? "https"),
        address: String(c.address ?? ""),
        port: Number(c.port ?? 32400),
        uri: String(c.uri ?? ""),
        local: Boolean(c.local),
        relay: Boolean(c.relay),
        ipv6: Boolean(c.IPv6 ?? c.ipv6),
      })),
    }));
}

/**
 * Pick a connection URI for a server based on declared preference, without
 * testing reachability. Order:
 *   1. LAN (local & not relay) — fast, but only reachable when the caller
 *      is on the same network.
 *   2. Public direct (not local, not relay) — works over the internet
 *      without going through Plex's relay infra.
 *   3. plex.direct relay — guaranteed to work, slowest.
 *
 * Use this for UI display where a "default" URI is fine. For actually
 * choosing what to talk to, use pickReachableConnectionUri so we don't
 * commit to a LAN URL that isn't routable from this host.
 */
export function pickConnectionUri(resource: PlexResource): string | null {
  const local = resource.connections.find((c) => c.local && !c.relay);
  if (local) return local.uri;
  const direct = resource.connections.find((c) => !c.local && !c.relay);
  if (direct) return direct.uri;
  const relay = resource.connections.find((c) => c.relay);
  return relay?.uri ?? null;
}

/**
 * Picks a usable connection URI for a server. The chosen URL carries
 * every Plex Media Server call we make — library data, posters, and
 * (most importantly) video streams — so we want the fastest path that's
 * actually reachable from where this app is running.
 *
 * Order, with per-tier timeouts tuned to fail fast on unreachable paths:
 *   1. LAN (`local: true`)        — fastest when this app shares a
 *                                    network with Plex. 1s timeout: any
 *                                    truly local server answers in
 *                                    well under that, so a miss means
 *                                    "different network, move on."
 *   2. Public direct (`local:false, relay:false`) — Plex's plex.direct
 *                                    pointed at the user's public IP,
 *                                    works whenever 32400 is forwarded.
 *                                    2.5s timeout for normal RTT
 *                                    variance.
 *   3. Relay (`relay: true`)      — Plex's hosted proxy, always
 *                                    reachable, capped bandwidth (2 Mbps
 *                                    free, ~5 Mbps with Plex Pass). Last
 *                                    resort, 5s timeout.
 *
 * Worst case: ~8s on a fully-misconfigured remote-access setup before
 * giving up. Best case (LAN reachable): <100ms.
 *
 * Probes hit /identity (no auth required, tiny response).
 */
export async function pickReachableConnectionUri(
  resource: PlexResource,
): Promise<string | null> {
  const tiers: Array<{ filter: (c: PlexConnection) => boolean; timeoutMs: number }> = [
    { filter: (c) => c.local && !c.relay, timeoutMs: 1000 },
    { filter: (c) => !c.local && !c.relay, timeoutMs: 2500 },
    { filter: (c) => c.relay, timeoutMs: 5000 },
  ];
  for (const tier of tiers) {
    for (const conn of resource.connections.filter(tier.filter)) {
      if (!isAllowedProbeUri(conn.uri)) continue;
      if (await isReachable(conn.uri, tier.timeoutMs)) return conn.uri;
    }
  }
  return null;
}

// Reject connection URIs that point at hosts the server shouldn't be
// fetching. plex.tv is trusted to return server addresses, but a
// compromised/MITMed resources response should not be able to coax us
// into hitting cloud metadata endpoints or loopback services with
// (later) authenticated requests.
//
// We allow plain RFC1918 IPs because Plex genuinely uses them for LAN
// connections — the goal is to block specifically dangerous addresses,
// not all private space.
export function isAllowedProbeUri(uri: string): boolean {
  let u: URL;
  try {
    u = new URL(uri);
  } catch {
    return false;
  }
  if (u.protocol !== "http:" && u.protocol !== "https:") return false;
  // Strip trailing dot ("localhost." resolves the same as "localhost"
  // but bypasses naive equality checks). Lowercase for case-insensitive
  // comparison.
  const host = u.hostname.toLowerCase().replace(/\.$/, "");
  if (isBlockedHostLiteral(host)) return false;
  return true;
}

// Literal-hostname / IP block check. Used by isAllowedProbeUri and by
// the DNS-resolution check that runs before we commit a URL to the
// session cookie.
function isBlockedHostLiteral(host: string): boolean {
  if (!host) return true;
  if (host === "localhost") return true;
  // IPv4 loopback / link-local / all-zeros.
  if (/^127\./.test(host)) return true;
  if (/^169\.254\./.test(host)) return true;
  if (host === "0.0.0.0") return true;
  // Plain numeric IPv4 — block obviously-private "single number" forms
  // like 2130706433 (== 127.0.0.1) that some libraries accept.
  if (/^\d+$/.test(host)) return true;
  // IPv6 forms. URL.hostname strips outer brackets, but be defensive.
  const v6 = host.replace(/^\[|\]$/g, "");
  if (v6 === "::1" || v6 === "::" || v6 === "0:0:0:0:0:0:0:1") return true;
  if (/^fe80:/i.test(v6)) return true;
  // IPv4-mapped IPv6: ::ffff:127.0.0.1 / ::ffff:7f00:1 — point at the
  // IPv4 stack. The hex form expands to the same v4 address.
  if (/^::ffff:/i.test(v6)) {
    const tail = v6.slice(7);
    // Dotted-quad form: ::ffff:127.0.0.1 — re-check the v4 piece.
    if (/^\d+\.\d+\.\d+\.\d+$/.test(tail) && isBlockedHostLiteral(tail))
      return true;
    // Hex form: ::ffff:7f00:1 — first hex group is the high byte.
    if (/^[0-9a-f]{1,4}(:[0-9a-f]{1,4})*$/i.test(tail)) {
      const groups = tail.split(":");
      const high = parseInt(groups[0] ?? "0", 16);
      if ((high & 0xff00) === 0x7f00) return true;
      if ((high & 0xffff) === 0xa9fe) return true; // 169.254.x.x
    }
  }
  return false;
}

// Resolves the URL's host to an IP and verifies the result isn't in the
// block-list. This is the defense against DNS-rebinding-style attacks
// where a hostname passes the literal allowlist but its A record points
// at 127.0.0.1. Use before any code path that will store the URL in a
// cookie and then send authenticated requests to it.
export async function assertProbeUriResolvesSafely(uri: string): Promise<void> {
  if (!isAllowedProbeUri(uri)) {
    throw new Error("rejected connection URI");
  }
  let u: URL;
  try {
    u = new URL(uri);
  } catch {
    throw new Error("rejected connection URI");
  }
  const host = u.hostname.toLowerCase().replace(/\.$/, "");
  // Skip DNS resolution for literal IPs — the literal check already
  // covered them.
  if (/^\d+\.\d+\.\d+\.\d+$/.test(host)) return;
  if (host.includes(":") || host.startsWith("[")) return;
  const dns = await import("node:dns/promises");
  let addrs: { address: string }[];
  try {
    addrs = await dns.lookup(host, { all: true });
  } catch {
    throw new Error("rejected connection URI: DNS lookup failed");
  }
  for (const a of addrs) {
    if (isBlockedHostLiteral(a.address.toLowerCase())) {
      throw new Error("rejected connection URI: resolves to blocked address");
    }
  }
}

async function isReachable(uri: string, timeoutMs: number): Promise<boolean> {
  const base = uri.replace(/\/+$/, "");
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), timeoutMs);
  try {
    const res = await fetch(`${base}/identity`, {
      signal: ctrl.signal,
      headers: { Accept: "application/json" },
      cache: "no-store",
      // Don't follow redirects: a malicious 3xx response could bounce us
      // to a denied host. The probe is just a liveness check.
      redirect: "manual",
    });
    return res.ok;
  } catch {
    return false;
  } finally {
    clearTimeout(timer);
  }
}

// ── Per-server data calls ────────────────────────────────────────────────

/**
 * Capabilities string reported to Plex so its transcoder knows what codecs
 * we can decode. hls.js + browser MSE handles H.264 / AAC reliably; we list
 * common audio codecs Plex might encounter in source media.
 */
const CLIENT_CAPABILITIES =
  "protocols=http-video,http-streaming-video,http-streaming-video-720p,http-streaming-video-1080p,http-mp4-streaming;" +
  "videoDecoders=h264{profile:high&resolution:1080&level:51};" +
  "audioDecoders=aac,mp3,ac3{bitrate:800000&channels:8}";

export type ServerAuth = {
  // Stable identifier (Plex's clientIdentifier for the server); used for
  // cache keys and to recognize the server across sessions.
  id: string;
  // Base URL — typically a plex.direct or LAN URI from `connections`.
  url: string;
  // Per-server access token. May or may not equal the master plex.tv
  // token — for shared servers it's a different token scoped to the share.
  accessToken: string;
};

export async function plexServer(
  path: string,
  auth: ServerAuth,
  init: RequestInit & { cacheSeconds?: number } = {},
): Promise<Response> {
  const { cacheSeconds, ...restInit } = init;
  const baseUrl = auth.url.replace(/\/+$/, "");
  const url = `${baseUrl}${path.startsWith("/") ? path : `/${path}`}`;
  const headers = new Headers(restInit.headers);
  // Plex defaults to XML when the client sends `Accept: */*`. We always want
  // JSON for our metadata/library calls; Plex serves m3u8 / ts from the
  // transcoder regardless of the Accept header, so this is safe.
  // Callers can override by setting their own Accept (e.g. the subtitle
  // endpoint asks for */*, since Plex's universal subtitle transcoder
  // refuses to serve text content when JSON is demanded).
  if (!headers.has("Accept")) {
    headers.set("Accept", "application/json");
  }
  headers.set("X-Plex-Token", auth.accessToken);
  headers.set("X-Plex-Client-Identifier", env.plexClientIdentifier());
  headers.set("X-Plex-Product", env.plexProductName());
  headers.set("X-Plex-Version", "0.1.0");
  headers.set("X-Plex-Platform", "Web");
  headers.set("X-Plex-Platform-Version", "1.0");
  headers.set("X-Plex-Device", "Web");
  headers.set("X-Plex-Device-Name", env.plexDeviceName());
  headers.set("X-Plex-Client-Capabilities", CLIENT_CAPABILITIES);

  // Per-token data should never be cached at the URL level (Next's data
  // cache is URL-keyed, which would mix users on shared servers). Our
  // in-memory cache layer handles per-(serverId, token) caching properly.
  const fetchInit: RequestInit & {
    next?: { revalidate?: number | false; tags?: string[] };
  } = {
    ...restInit,
    headers,
  };
  if (cacheSeconds && cacheSeconds > 0) {
    fetchInit.next = { revalidate: cacheSeconds };
  } else {
    fetchInit.cache = "no-store";
  }
  try {
    return await fetch(url, fetchInit);
  } catch (e) {
    // Network/DNS/TLS failures from Node's fetch are otherwise opaque in
    // production logs (no stack, just a digest in the UI). Log here so
    // the operator can see exactly which Plex URL failed and why.
    // eslint-disable-next-line no-console
    console.error(
      `[plex] ${restInit.method ?? "GET"} ${url} — ${
        e instanceof Error ? e.message : String(e)
      }`,
    );
    throw e;
  }
}
