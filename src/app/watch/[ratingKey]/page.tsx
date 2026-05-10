import { notFound } from "next/navigation";
import {
  getChildren,
  getMetadata,
  type ServerAuth,
} from "@/lib/plex-data";
import { requireServerAuth } from "@/lib/session";
import { displayTitle, type MediaItem } from "@/lib/plex-types";
import { Player } from "@/components/Player";

/**
 * Resolves the next episode after the current one. Walks within a season
 * first, then falls back to the first episode of the next season. Returns
 * null for movies and the last episode of the last season.
 */
async function findNextEpisode(
  auth: ServerAuth,
  episode: MediaItem,
): Promise<MediaItem | null> {
  if (episode.type !== "episode") return null;

  const seasonKey = episode.parentRatingKey;
  if (seasonKey) {
    const episodes = await getChildren(auth, seasonKey);
    const idx = episodes.findIndex(
      (e) => e.ratingKey === episode.ratingKey,
    );
    if (idx >= 0 && idx + 1 < episodes.length) {
      return episodes[idx + 1];
    }
  }

  const showKey = episode.grandparentRatingKey;
  if (!showKey || !seasonKey) return null;
  const seasons = await getChildren(auth, showKey);
  const seasonIdx = seasons.findIndex((s) => s.ratingKey === seasonKey);
  if (seasonIdx >= 0 && seasonIdx + 1 < seasons.length) {
    const nextSeasonEpisodes = await getChildren(
      auth,
      seasons[seasonIdx + 1].ratingKey,
    );
    return nextSeasonEpisodes[0] ?? null;
  }
  return null;
}

/**
 * Plex's /transcode endpoint expects a leaf media item (movie / episode /
 * track). Shows and seasons aren't directly playable — for those we walk
 * down to the first episode. Continue Watching / onDeck-aware resolution
 * can come later.
 */
async function resolvePlayable(
  auth: ServerAuth,
  ratingKey: string,
): Promise<MediaItem | null> {
  const item = await getMetadata(auth, ratingKey);
  if (!item) return null;

  if (item.type === "movie" || item.type === "episode") return item;

  if (item.type === "show") {
    const seasons = await getChildren(auth, ratingKey);
    for (const season of seasons) {
      const episodes = await getChildren(auth, season.ratingKey);
      if (episodes[0]) return episodes[0];
    }
    return null;
  }

  if (item.type === "season") {
    const episodes = await getChildren(auth, ratingKey);
    return episodes[0] ?? null;
  }

  return null;
}

export default async function WatchPage({
  params,
}: {
  params: Promise<{ ratingKey: string }>;
}) {
  const { ratingKey } = await params;
  const auth = await requireServerAuth();

  const item = await resolvePlayable(auth, ratingKey);
  if (!item) notFound();

  const backRatingKey =
    item.type === "episode" && item.grandparentRatingKey
      ? item.grandparentRatingKey
      : ratingKey;

  const nextEpisode = await findNextEpisode(auth, item);

  const seasonEpisodes =
    item.type === "episode" && item.parentRatingKey
      ? await getChildren(auth, item.parentRatingKey)
      : [];

  return (
    <Player
      ratingKey={item.ratingKey}
      title={displayTitle(item)}
      subtitle={
        item.type === "episode" && item.parentTitle
          ? `${item.parentTitle} · ${item.title}`
          : undefined
      }
      duration={item.duration}
      viewOffset={item.viewOffset}
      backToRatingKey={backRatingKey}
      nextRatingKey={nextEpisode?.ratingKey}
      nextLabel={nextEpisode?.title}
      nextThumb={nextEpisode?.thumb}
      markers={item.markers}
      seasonEpisodes={seasonEpisodes.map((e) => ({
        ratingKey: e.ratingKey,
        title: e.title,
        thumb: e.thumb,
        summary: e.summary,
        duration: e.duration,
        viewOffset: e.viewOffset,
        index: e.index,
        parentTitle: e.parentTitle,
      }))}
    />
  );
}
