import { NextResponse } from "next/server";
import { readHiddenLibraries } from "@/lib/library-prefs";
import {
  getChildren,
  getMetadata,
  getSimilar,
  type MediaItem,
} from "@/lib/plex-data";
import { resolveServerAuth } from "@/lib/session";

// Single bundled endpoint for modal data. Replaces 4 separate browser
// fetches with one server-side roundtrip that goes through the in-memory
// cache, so a second open of the same modal is essentially free.
export async function GET(
  _req: Request,
  ctx: { params: Promise<{ ratingKey: string }> },
) {
  const auth = await resolveServerAuth();
  if (!auth) {
    return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  }
  const { ratingKey } = await ctx.params;

  const [item, similarRaw, children, hidden] = await Promise.all([
    getMetadata(auth, ratingKey),
    getSimilar(auth, ratingKey).catch(() => [] as MediaItem[]),
    getChildren(auth, ratingKey).catch(() => [] as MediaItem[]),
    readHiddenLibraries(),
  ]);
  if (!item) {
    return NextResponse.json({ error: "not found" }, { status: 404 });
  }

  const similar = similarRaw.filter(
    (it) =>
      it.librarySectionID === undefined ||
      !hidden.has(String(it.librarySectionID)),
  );
  const seasons = item.type === "show" ? children : [];
  let initialEpisodes: MediaItem[] = [];
  if (seasons[0]) {
    initialEpisodes = await getChildren(auth, seasons[0].ratingKey).catch(
      () => [] as MediaItem[],
    );
  }

  return NextResponse.json({ item, seasons, initialEpisodes, similar });
}
