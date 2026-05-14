import { NextRequest, NextResponse } from "next/server";

// Defense-in-depth CSRF protection for side-effecting API routes.
//
// SameSite=lax on our session cookies already blocks the classic
// cross-site form POST, but it does NOT block:
//   - Same-site cross-origin attacks (a stored XSS on a sibling
//     subdomain initiating a fetch with credentials).
//   - Older browsers without strong sameSite enforcement.
//
// So for any non-idempotent verb on /api/auth/** and other state-
// changing routes, require the Origin header to match the canonical
// origin. fetch() always sends Origin, so legitimate clients are fine;
// CSRF attempts from a different origin get a 403.
export function middleware(req: NextRequest) {
  if (!isProtectedPath(req.nextUrl.pathname)) return NextResponse.next();
  if (isSafeMethod(req.method)) return NextResponse.next();

  const origin = req.headers.get("origin");
  // No Origin header on a POST is suspicious. Modern browsers set it on
  // every cross-origin request and on most same-origin ones from fetch.
  // Server-to-server callers don't hit this middleware (they use the API
  // routes directly from Node), so missing Origin is reject-worthy.
  if (!origin) {
    return NextResponse.json({ error: "missing origin" }, { status: 403 });
  }

  const expected = expectedOrigin(req);
  if (!expected) return NextResponse.next();
  if (origin.replace(/\/+$/, "") !== expected.replace(/\/+$/, "")) {
    return NextResponse.json({ error: "bad origin" }, { status: 403 });
  }
  return NextResponse.next();
}

function isProtectedPath(pathname: string): boolean {
  return (
    pathname.startsWith("/api/auth/") ||
    pathname.startsWith("/api/prefs/") ||
    pathname.startsWith("/api/plex/")
  );
}

function isSafeMethod(method: string): boolean {
  return method === "GET" || method === "HEAD" || method === "OPTIONS";
}

function expectedOrigin(req: NextRequest): string | null {
  // APP_PUBLIC_ORIGIN is the canonical answer when configured.
  const configured = process.env.APP_PUBLIC_ORIGIN?.trim();
  if (configured) return configured.replace(/\/+$/, "");
  // Fall back to the request's own Host. Note: we don't trust
  // X-Forwarded-Proto here without APP_TRUST_PROXY — defaulting to "http"
  // is safe because both legitimate requests and attempted CSRF use the
  // same scheme to reach us.
  const host = req.headers.get("host");
  if (!host) return null;
  const proto =
    process.env.APP_TRUST_PROXY === "1"
      ? req.headers.get("x-forwarded-proto")?.split(",")[0]?.trim() || "http"
      : "http";
  return `${proto}://${host}`;
}

export const config = {
  // Match the protected API namespace. Don't run middleware on every
  // request — RSC navigation and static asset fetches would pay an
  // unnecessary cost.
  matcher: ["/api/auth/:path*", "/api/prefs/:path*", "/api/plex/:path*"],
};
