import { NextResponse } from "next/server";
import {
  ChimpFlixApiError,
  items as itemsApi,
  seasons as seasonsApi,
} from "@/lib/chimpflix-api";
import {
  adaptEpisode,
  adaptItem,
  adaptItemDetail,
  adaptSeason,
} from "@/lib/chimpflix-adapt";
import type { MediaItem } from "@/lib/chimpflix-types";

// Modal data, now sourced from the Rust backend. Shape preserved so the
// client (TitleModalClient + modal-cache) doesn't need to change. `similar`
// is empty until the backend grows a recommender; the UI hides the rail
// when it's empty.
export async function GET(
  _req: Request,
  ctx: { params: Promise<{ ratingKey: string }> },
) {
  const { ratingKey } = await ctx.params;
  const id = Number.parseInt(ratingKey, 10);
  if (!Number.isFinite(id) || id <= 0) {
    return NextResponse.json({ error: "bad id" }, { status: 400 });
  }

  try {
    const detail = await itemsApi.get(id);
    const item = adaptItemDetail(detail);
    const seasonMediaItems = detail.seasons.map((s) => adaptSeason(s, detail));

    let initialEpisodes: MediaItem[] = [];
    const firstSeason = detail.seasons[0];
    if (firstSeason) {
      try {
        const seasonDetail = await seasonsApi.get(firstSeason.id);
        initialEpisodes = seasonDetail.episodes.map((ep) => {
          const base = adaptEpisode(ep, detail);
          if (ep.play_state && ep.play_state.position_ms > 0) {
            base.viewOffset = ep.play_state.position_ms;
          }
          if (ep.play_state?.watched) {
            base.watched = true;
          }
          return base;
        });
      } catch {
        // Best-effort: leave empty if the season fetch fails.
      }
    }

    let similar: MediaItem[] = [];
    try {
      const similarResp = await itemsApi.similar(id);
      similar = similarResp.items.map(adaptItem);
    } catch {
      // Similar is best-effort — TMDB outage shouldn't blow up the modal.
    }

    return NextResponse.json({
      item,
      seasons: seasonMediaItems,
      initialEpisodes,
      similar,
      // Pass through the new ItemDetail fields. The client renders
      // Cast & Crew, Extras, and Reviews sections directly from these.
      credits: detail.credits ?? [],
      extras: detail.extras ?? [],
      reviews: detail.reviews ?? { count: 0, average: null },
      locked_fields: detail.locked_fields ?? [],
      // The raw ItemDetail is included so Edit / Fix Match dialogs have
      // every original field without needing a second round-trip.
      detail,
    });
  } catch (e) {
    if (e instanceof ChimpFlixApiError) {
      if (e.status === 401) {
        return NextResponse.json({ error: "unauthorized" }, { status: 401 });
      }
      if (e.status === 404) {
        return NextResponse.json({ error: "not found" }, { status: 404 });
      }
    }
    console.error("modal route error:", e);
    return NextResponse.json({ error: "internal" }, { status: 500 });
  }
}
