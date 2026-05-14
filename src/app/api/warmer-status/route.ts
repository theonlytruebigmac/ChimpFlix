import { NextResponse } from "next/server";
import { isWarmerReady } from "@/lib/cache-warmer";

// Force dynamic so Next.js doesn't statically cache the response — we
// read mutable module state (firstTickDone) that flips exactly once
// per process lifecycle, and a cached `false` would never recover.
export const dynamic = "force-dynamic";

/**
 * Lightweight liveness check for the cache-warmer's first cycle. The
 * cold-start overlay polls this so it can dismiss itself the moment
 * the global rails are populated, without us having to round-trip the
 * whole page render to know.
 *
 * Cheap (no Plex calls, no auth) so polling at 2s is fine.
 */
export async function GET() {
  return NextResponse.json(
    { ready: isWarmerReady() },
    {
      headers: {
        // Never cache — the value is meant to change exactly once per
        // process lifecycle and clients need the change reflected
        // immediately.
        "Cache-Control": "no-store",
      },
    },
  );
}
