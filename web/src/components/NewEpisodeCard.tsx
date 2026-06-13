import Link from "next/link";
import type { RecentEpisode } from "@/lib/chimpflix-api";
import { plexImage } from "@/lib/image";
import { formatRelativeAgo } from "@/lib/relative-time";

/// "S1 · E9 — Title" style episode line — same format as the calendar card.
/// Season/episode numbers are shown un-padded (matches SeasonEpisodes); the
/// title is optional (the backend normalizes synthetic "Episode N"
/// placeholders to null, so we only render the dash + title when present).
function episodeLine(ep: RecentEpisode): string {
  const code = `S${ep.seasonNumber} · E${ep.episodeNumber}`;
  return ep.episodeTitle ? `${code} — ${ep.episodeTitle}` : code;
}

/// A single "New Episode" tile — a fresh episode of a show the user already
/// had. Same visual shape as {@link CalendarEpisodeCard} (episode still →
/// show backdrop → poster fallback, links into the show), but episode-centric
/// rather than air-date-centric: it carries a "NEW" badge and an "added X ago"
/// label off `addedAt` (when the file landed) instead of an air-date label.
///
/// `nowMs` is threaded in (not read from the wall clock) so every tile on the
/// rail shares one reference instant — the codebase's relative-time purity
/// convention (avoids SSR/CSR hydration drift).
export function NewEpisodeCard({
  episode,
  nowMs,
  className = "",
}: {
  episode: RecentEpisode;
  nowMs: number;
  className?: string;
}) {
  const thumbPath =
    episode.stillPath ?? episode.backdropPath ?? episode.posterPath ?? undefined;
  const img = plexImage(thumbPath, 480, 270);
  const added = formatRelativeAgo(episode.addedAt, nowMs);

  return (
    <Link
      // Open the show the same way every other card does — /watch/{showId};
      // the player resolves a show id to its season/episode picker. (Episodes
      // have no standalone route.)
      href={`/watch/${episode.showId}`}
      aria-label={`${episode.showTitle} — ${episodeLine(episode)} — added ${added}`}
      className={
        // transform-gpu + will-change + hidden backface promote the card to its
        // own GPU layer so animating `scale` doesn't show compositor seams
        // through the rounded, clipped image — matches CalendarEpisodeCard.
        "group relative block overflow-hidden rounded-md bg-(--color-surface) transition-transform backface-hidden transform-gpu will-change-transform hover:scale-[1.04] focus:scale-[1.04] focus:outline-none focus-visible:ring-2 focus-visible:ring-accent " +
        className
      }
    >
      <div className="aspect-video w-full bg-(--color-surface-elevated)">
        {img && (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={img}
            alt={episode.showTitle}
            width={480}
            height={270}
            loading="lazy"
            className="block h-full w-full object-cover backface-hidden transform-gpu"
          />
        )}
      </div>
      <div className="pointer-events-none absolute left-0 top-2 select-none rounded-r-sm bg-(--color-accent) px-2 py-1 text-[0.65rem] font-bold uppercase leading-none tracking-wide text-white shadow-md">
        New
      </div>
      <div className="absolute inset-x-0 bottom-0 bg-linear-to-t from-black/90 to-transparent p-3">
        <div className="line-clamp-1 text-sm font-semibold">
          {episode.showTitle}
        </div>
        <div className="line-clamp-1 text-xs text-neutral-300">
          {episodeLine(episode)}
        </div>
        <div className="mt-1 text-[11px] font-medium uppercase tracking-wide text-accent">
          added {added}
        </div>
      </div>
    </Link>
  );
}
