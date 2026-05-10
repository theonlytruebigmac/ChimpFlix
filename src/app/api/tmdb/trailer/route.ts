import { NextRequest, NextResponse } from "next/server";
import { getOrFetch } from "@/lib/cache";
import { findTrailerVideoId } from "@/lib/tmdb";
import { readToken } from "@/lib/session";

const TTL_TRAILER = 24 * 60 * 60_000; // 24h — trailers rarely change

export async function GET(req: NextRequest) {
  // Require an authenticated session — keeps our TMDB token from being used
  // by random callers and lets us assume a Plex login on the way through.
  const session = await readToken();
  if (!session) {
    return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  }

  const sp = req.nextUrl.searchParams;
  const type = sp.get("type");
  const title = sp.get("title")?.trim();
  const yearStr = sp.get("year");

  if (type !== "movie" && type !== "tv") {
    return NextResponse.json(
      { error: "type must be 'movie' or 'tv'" },
      { status: 400 },
    );
  }
  if (!title) {
    return NextResponse.json({ error: "missing title" }, { status: 400 });
  }

  const year = yearStr ? Number(yearStr) : undefined;
  // Cache by (type|title|year) tuple. Trailers for a given title+year don't
  // change day-to-day, and modal opens on the same title hit this endpoint
  // every time. Caching here saves both the TMDB roundtrip and the YouTube
  // search behind it.
  const cacheKey = `tmdbTrailer:${type}:${title.toLowerCase()}:${year ?? ""}`;
  const videoId = await getOrFetch<string | null>(
    cacheKey,
    () =>
      findTrailerVideoId(
        type,
        title,
        Number.isFinite(year) ? year : undefined,
      ),
    { ttlMs: TTL_TRAILER },
  );
  // Hint the browser to cache the JSON too — trailers are stable enough that
  // the browser holding it for an hour saves another roundtrip on revisit.
  return NextResponse.json(
    { videoId },
    {
      headers: {
        "Cache-Control":
          "private, max-age=3600, stale-while-revalidate=86400",
      },
    },
  );
}
