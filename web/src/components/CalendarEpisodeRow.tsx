import Link from "next/link";
import { plexImage } from "@/lib/image";
import { CalendarAvailability } from "@/components/CalendarAvailability";
import type { CalendarEpisode } from "@/lib/chimpflix-api";

/// "S1 · E9 — Title" episode line (un-padded, matches SeasonEpisodes); the
/// title is omitted when the backend normalized a synthetic "Episode N"
/// placeholder to null.
function episodeLine(ep: CalendarEpisode): string {
  const code = `S${ep.seasonNumber} · E${ep.episodeNumber}`;
  return ep.episodeTitle ? `${code} — ${ep.episodeTitle}` : code;
}

/// A compact horizontal agenda row — a small landscape thumbnail, the show +
/// episode line, and a Finale / Premiere tag. Used by the /calendar page's
/// "Up Next" and "Recently aired" lists (the day is already the group heading,
/// so the row carries no relative-day label). Links into the show the same way
/// every other card does — /watch/{showId} (there is no /show/[id] route).
export function CalendarEpisodeRow({ episode }: { episode: CalendarEpisode }) {
  const thumbPath =
    episode.stillPath ?? episode.backdropPath ?? episode.posterPath ?? undefined;
  const img = plexImage(thumbPath, 240, 135);
  const tag = episode.isFinale
    ? { label: "Finale", cls: "bg-[#ad2cff]/18 text-[#d9a6ff]" }
    : episode.isPremiere
      ? { label: "Premiere", cls: "bg-accent/15 text-[#ff8b8b]" }
      : null;

  return (
    <Link
      href={`/watch/${episode.showId}`}
      aria-label={`${episode.showTitle} — ${episodeLine(episode)}`}
      className="group flex items-center gap-3.5 rounded-lg p-2 transition-colors hover:bg-(--color-surface) focus:bg-(--color-surface) focus:outline-none focus-visible:ring-2 focus-visible:ring-accent sm:gap-4"
    >
      <div className="aspect-video w-21 shrink-0 overflow-hidden rounded bg-(--color-surface-elevated) sm:w-24">
        {img && (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={img}
            alt={episode.showTitle}
            width={240}
            height={135}
            loading="lazy"
            className="block h-full w-full object-cover"
          />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="line-clamp-1 text-sm font-semibold">
          {episode.showTitle}
        </div>
        <div className="line-clamp-1 text-xs text-white/55">
          {episodeLine(episode)}
        </div>
      </div>
      <div className="ml-auto flex shrink-0 items-center gap-2.5">
        {tag && (
          <span
            className={
              "rounded-full px-2.5 py-1 text-[10px] font-bold uppercase tracking-wide " +
              tag.cls
            }
          >
            {tag.label}
          </span>
        )}
        <CalendarAvailability hasFile={episode.hasFile} />
      </div>
    </Link>
  );
}
