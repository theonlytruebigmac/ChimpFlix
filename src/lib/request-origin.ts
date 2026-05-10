import { env } from "./env";

// Resolve the canonical origin to use in same-origin redirects (login
// callbacks, post-logout bounces, etc).
//
// Order of trust:
//   1. APP_PUBLIC_ORIGIN env — authoritative if set. Use this in any
//      Internet-facing deployment so attacker-supplied Host headers
//      can't redirect users to arbitrary sites.
//   2. Host header — fine for LAN/dev deployments. We honor
//      X-Forwarded-Proto only when APP_TRUST_PROXY=1 so a direct
//      connection to Node can't spoof https.
//   3. req.url — last resort. Reflects the bind address (often
//      0.0.0.0 in Docker) and is not browser-reachable.
export function originFromRequest(req: Request): string {
  const configured = env.publicOrigin();
  if (configured) return configured;
  const host = req.headers.get("host");
  if (host) {
    const xfp = env.trustProxy()
      ? req.headers.get("x-forwarded-proto")?.split(",")[0]?.trim()
      : undefined;
    const proto = xfp || "http";
    return `${proto}://${host}`;
  }
  return new URL(req.url).origin;
}
