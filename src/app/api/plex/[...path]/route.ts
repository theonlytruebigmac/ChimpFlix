import { NextRequest, NextResponse } from "next/server";
import { plexServer } from "@/lib/plex";
import { resolveServerAuth } from "@/lib/session";

const HOP_BY_HOP = new Set([
  "connection",
  "keep-alive",
  "transfer-encoding",
  "upgrade",
  "proxy-authenticate",
  "proxy-authorization",
  "te",
  "trailers",
  "host",
  "content-length",
]);

// Headers that must NOT be forwarded back to the browser. `content-encoding`
// is dropped because Node's fetch transparently decompresses the upstream body
// — if we leave the header on the response, the browser tries to decompress
// the already-decompressed bytes and image/JSON decoding fails.
const STRIP_RESPONSE = new Set([...HOP_BY_HOP, "content-encoding"]);

async function handle(
  req: NextRequest,
  ctx: RouteContext<"/api/plex/[...path]">,
) {
  const auth = await resolveServerAuth();
  if (!auth) {
    return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  }

  const { path } = await ctx.params;
  const segments = Array.isArray(path) ? path : [path];
  // Plex paths can include literal ':' (e.g. /photo/:/transcode) so don't
  // URL-encode segments — Next has already decoded them once for us.
  const pathname = "/" + segments.join("/");
  const search = req.nextUrl.search;

  // CSRF protection for state-changing Plex paths that are called with
  // GET (Plex's own URL conventions — :/timeline updates playback
  // progress, :/scrobble marks watched, :/unscrobble marks unwatched,
  // /refresh kicks library scans). An attacker can't read the response
  // cross-origin, but a forged "watched" marker via <img src> is enough
  // to cause harm. Block any request whose Origin doesn't match.
  //
  // GETs that are read-only (images, library listings, search) skip
  // this check so <img> tags and prefetches keep working. HEAD/OPTIONS
  // can't carry side effects.
  if (
    req.method !== "HEAD" &&
    req.method !== "OPTIONS" &&
    isStateChangingPlexPath(pathname) &&
    !originIsSameSite(req)
  ) {
    return NextResponse.json({ error: "bad origin" }, { status: 403 });
  }

  const init: RequestInit = {
    method: req.method,
    headers: filterRequestHeaders(req.headers),
    // Propagate client-side aborts upstream so Plex can free its transcoder
    // session cleanly when hls.js cancels a segment fetch.
    signal: req.signal,
  };
  if (req.method !== "GET" && req.method !== "HEAD") {
    init.body = req.body;
    // @ts-expect-error -- node fetch requires duplex for streamed bodies
    init.duplex = "half";
  }

  let upstream: Response;
  try {
    upstream = await plexServer(pathname + search, auth, init);
  } catch (e) {
    // Aborted requests are expected (hls.js cancels segments routinely);
    // turn them into a 499-ish empty response instead of a crash.
    if (e instanceof Error && e.name === "AbortError") {
      return new Response(null, { status: 499 });
    }
    throw e;
  }

  const headers = filterResponseHeaders(upstream.headers);

  // Browser-cache image bytes aggressively. Plex's art URLs are versioned by
  // a timestamp in the path (e.g. /library/metadata/12345/art/1700000000), so
  // a given URL's bytes never change — when the underlying art is replaced
  // the URL changes too. That makes `immutable` safe and pulls the entire
  // catalog of thumbs into the browser's disk cache for repeat visits.
  // `private` keeps shared caches/CDNs out of it since the bytes are gated
  // by a Plex token.
  const upstreamContentType = upstream.headers.get("content-type") ?? "";
  if (upstreamContentType.startsWith("image/") && upstream.ok) {
    headers.set(
      "Cache-Control",
      "private, max-age=2592000, immutable",
    );
  }

  // Use plain Response instead of NextResponse for binary stream forwarding —
  // NextResponse's body handling has been flaky for long-lived transcoder
  // streams (segments mid-flight closing as "failed to pipe response").
  return new Response(upstream.body, {
    status: upstream.status,
    statusText: upstream.statusText,
    headers,
  });
}

// Plex endpoints that mutate server-side state. Each is reachable by
// GET because Plex's API predates REST conventions, so a cross-origin
// `<img>` tag could fire them with the user's session — hence the
// explicit Origin check.
//
// Normalization is important here: an attacker who can encode the
// path (`%3A/timeline`), case-shift it (`/:/Timeline`), or wrap it in
// traversal (`/foo/../:/timeline`) shouldn't sneak past the prefix
// check while still hitting Plex's canonical path.
function isStateChangingPlexPath(pathname: string): boolean {
  // Reject any path that contains traversal segments — they have no
  // legitimate use in a Plex API call and shouldn't be forwarded.
  if (/(^|\/)\.\.(\/|$)/.test(pathname)) return true;
  let decoded = pathname;
  // Decode percent-encoded sequences a couple of times in case of
  // double-encoding. URIError on invalid sequences just stops decoding.
  for (let i = 0; i < 2; i++) {
    try {
      const next = decodeURIComponent(decoded);
      if (next === decoded) break;
      decoded = next;
    } catch {
      break;
    }
  }
  const norm = decoded.replace(/^\/+/, "").toLowerCase();
  return (
    norm.startsWith(":/timeline") ||
    norm.startsWith(":/scrobble") ||
    norm.startsWith(":/unscrobble") ||
    norm.startsWith(":/rate") ||
    norm.startsWith(":/progress") ||
    /\/refresh(\?|$|\/)/.test(decoded.toLowerCase()) ||
    /\/unmatch(\?|$|\/)/.test(decoded.toLowerCase()) ||
    /\/match(\?|$|\/)/.test(decoded.toLowerCase())
  );
}

function originIsSameSite(req: NextRequest): boolean {
  const origin = req.headers.get("origin");
  if (!origin) return false;
  const configured = process.env.APP_PUBLIC_ORIGIN?.trim()?.replace(/\/+$/, "");
  if (configured) return origin.replace(/\/+$/, "") === configured;
  const host = req.headers.get("host");
  if (!host) return false;
  const proto =
    process.env.APP_TRUST_PROXY === "1"
      ? req.headers.get("x-forwarded-proto")?.split(",")[0]?.trim() || "http"
      : "http";
  return origin.replace(/\/+$/, "") === `${proto}://${host}`;
}

function filterRequestHeaders(src: Headers): Headers {
  const out = new Headers();
  src.forEach((value, key) => {
    const k = key.toLowerCase();
    if (HOP_BY_HOP.has(k)) return;
    if (k.startsWith("x-plex-")) return;
    // Never forward the browser's Cookie header upstream. It carries
    // our own signed session material (cf_token, cf_admin, cf_server)
    // which Plex doesn't act on but would log/store — and if the
    // upstream URL is ever wrong, those cookies leak to a third party.
    if (k === "cookie") return;
    out.set(key, value);
  });
  return out;
}

function filterResponseHeaders(src: Headers): Headers {
  const out = new Headers();
  src.forEach((value, key) => {
    if (STRIP_RESPONSE.has(key.toLowerCase())) return;
    out.set(key, value);
  });
  return out;
}

export const GET = handle;
export const HEAD = handle;
export const POST = handle;
export const PUT = handle;
export const PATCH = handle;
export const DELETE = handle;
export const OPTIONS = handle;
