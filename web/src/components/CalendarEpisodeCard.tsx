import Link from "next/link";
import { plexImage } from "@/lib/image";
import { relativeDayLabel } from "@/lib/relative-time";
import { CalendarAvailability } from "@/components/CalendarAvailability";
import type { CalendarEpisode } from "@/lib/chimpflix-api";

/// "S1 · E9 — Title" style episode line. Season/episode numbers are shown
/// un-padded (matches the app's SeasonEpisodes rendering); the title is
/// optional (the backend normalizes synthetic "Episode N" placeholders to
/// null, so we only render the dash + title when we actually have one).
function episodeLine(ep: CalendarEpisode): string {
  const code = `S${ep.seasonNumber} · E${ep.episodeNumber}`;
  return ep.episodeTitle ? `${code} — ${ep.episodeTitle}` : code;
}

/// A single upcoming-episode tile. Picks the episode still when scanned,
/// else the parent show's backdrop, else its poster. Links into the show
/// detail page (episodes don't have their own route). Used by both the
/// home "Coming up" rail and the /calendar grid, so the visual matches.
///
/// `nowMs` is threaded in (not read from the wall clock) so every tile on a
/// surface shares one reference instant — same purity convention as the rest
/// of the relative-time helpers.
export function CalendarEpisodeCard({
  episode,
  nowMs,
  showWhen = true,
  className = "",
}: {
  episode: CalendarEpisode;
  nowMs: number;
  /// Show the relative-day label badge. The /calendar page groups by day
  /// (so the day is already the section heading) and turns this off; the
  /// home rail leaves it on.
  showWhen?: boolean;
  className?: string;
}) {
  const thumbPath =
    episode.stillPath ?? episode.backdropPath ?? episode.posterPath ?? undefined;
  const img = plexImage(thumbPath, 480, 270);
  const when = relativeDayLabel(episode.airDate, nowMs);
  const badge = episode.isFinale
    ? "Finale"
    : episode.isPremiere
      ? "Premiere"
      : null;

  return (
    <Link
      // Open the show the same way every other card does — /watch/{ratingKey},
      // where a show's ratingKey is just its item id (see chimpflix-adapt).
      // There is no /show/[id] route; the player resolves a show id to its
      // season/episode picker. (Linking the episode would be wrong for
      // placeholders, which have no file to play.)
      href={`/watch/${episode.showId}`}
      aria-label={`${episode.showTitle} — ${episodeLine(episode)}`}
      className={
        // `transform-gpu` + will-change + hidden backface promote the card to
        // its own GPU layer up-front, so animating `scale` doesn't show thin
        // compositor seams ("white lines") through the rounded, clipped image.
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
      {badge && (
        <div className="pointer-events-none absolute left-0 top-2 select-none rounded-r-sm bg-(--color-accent) px-2 py-1 text-[0.65rem] font-bold uppercase leading-none tracking-wide text-white shadow-md">
          {badge}
        </div>
      )}
      <div className="absolute inset-x-0 bottom-0 bg-linear-to-t from-black/90 to-transparent p-3">
        <div className="line-clamp-1 text-sm font-semibold">
          {episode.showTitle}
        </div>
        <div className="line-clamp-1 text-xs text-neutral-300">
          {episodeLine(episode)}
        </div>
        <div className="mt-1 flex items-center gap-2">
          {showWhen && (
            <span className="text-[11px] font-medium uppercase tracking-wide text-accent">
              {when}
            </span>
          )}
          <CalendarAvailability
            hasFile={episode.hasFile}
            className={showWhen ? "ml-auto" : ""}
          />
        </div>
      </div>
    </Link>
  );
}
