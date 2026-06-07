"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { plexImage } from "@/lib/image";
import { airDayKey } from "@/lib/relative-time";
import type { CalendarEpisode } from "@/lib/chimpflix-api";

const DAY_MS = 86_400_000;

/// "Week Board" calendar view — a true 7-column Sun–Sat week grid (the
/// "real calendar" alternative to the spotlight agenda), with prev/next week
/// paging. Today's column is outlined and past days are dimmed.
///
/// All day math runs in UTC-midnight "anchor" space: `air_date` is a
/// midnight-UTC plain date, so columns are anchored to `Date.UTC(...)` and
/// matched to episodes by `airDayKey` (also UTC). "Today" is the viewer's
/// LOCAL current date projected into that same anchor space, so the highlight
/// tracks the viewer's calendar day, not UTC's. Working purely in UTC anchors
/// also sidesteps DST (UTC has no DST, so `anchor + n*DAY_MS` stays at
/// midnight).
export function CalendarWeekBoard({
  episodes,
  nowMs,
}: {
  episodes: CalendarEpisode[];
  nowMs: number;
}) {
  const [weekOffset, setWeekOffset] = useState(0);

  const byDay = useMemo(() => {
    const m = new Map<string, CalendarEpisode[]>();
    for (const ep of episodes) {
      const key = airDayKey(ep.airDate);
      const arr = m.get(key);
      if (arr) arr.push(ep);
      else m.set(key, [ep]);
    }
    return m;
  }, [episodes]);

  // Viewer's local today as a UTC-midnight anchor, and the Sunday that starts
  // the currently-displayed week.
  const n = new Date(nowMs);
  const todayAnchor = Date.UTC(n.getFullYear(), n.getMonth(), n.getDate());
  const todayDow = new Date(todayAnchor).getUTCDay(); // 0 = Sunday
  const weekStart = todayAnchor - todayDow * DAY_MS + weekOffset * 7 * DAY_MS;

  const days = Array.from({ length: 7 }, (_, i) => {
    const anchor = weekStart + i * DAY_MS;
    const d = new Date(anchor);
    return {
      anchor,
      dow: d.toLocaleDateString(undefined, {
        weekday: "short",
        timeZone: "UTC",
      }),
      dnum: d.getUTCDate(),
      episodes: byDay.get(airDayKey(anchor)) ?? [],
      isToday: anchor === todayAnchor,
      isPast: anchor < todayAnchor,
    };
  });

  const rangeStart = new Date(weekStart).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });
  const rangeEnd = new Date(weekStart + 6 * DAY_MS).toLocaleDateString(
    undefined,
    { month: "short", day: "numeric", year: "numeric", timeZone: "UTC" },
  );

  return (
    <div>
      <div className="mb-4 flex items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <button
            type="button"
            aria-label="Previous week"
            onClick={() => setWeekOffset((o) => o - 1)}
            className="flex h-9 w-9 items-center justify-center rounded-lg border border-white/10 bg-(--color-surface) text-lg leading-none text-white/80 transition-colors hover:bg-(--color-surface-elevated) focus:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          >
            ‹
          </button>
          <span className="min-w-44 text-center text-base font-semibold">
            {rangeStart} – {rangeEnd}
          </span>
          <button
            type="button"
            aria-label="Next week"
            onClick={() => setWeekOffset((o) => o + 1)}
            className="flex h-9 w-9 items-center justify-center rounded-lg border border-white/10 bg-(--color-surface) text-lg leading-none text-white/80 transition-colors hover:bg-(--color-surface-elevated) focus:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          >
            ›
          </button>
        </div>
        {weekOffset !== 0 && (
          <button
            type="button"
            onClick={() => setWeekOffset(0)}
            className="text-sm font-medium text-white/60 transition-colors hover:text-accent focus:outline-none focus-visible:text-accent"
          >
            Today
          </button>
        )}
      </div>

      {/* Horizontal scroll on narrow screens so the 7 columns stay legible. */}
      <div className="overflow-x-auto overscroll-x-contain pb-2">
        <div className="grid min-w-[760px] grid-cols-7 gap-2.5">
          {days.map((day) => (
            <div
              key={day.anchor}
              className={
                "min-h-72 overflow-hidden rounded-xl border bg-(--color-surface) " +
                (day.isToday
                  ? "border-accent/50 ring-1 ring-accent/25"
                  : "border-white/10") +
                (day.isPast ? " opacity-55" : "")
              }
            >
              <div
                className={
                  "border-b px-2 py-2.5 text-center " +
                  (day.isToday
                    ? "border-white/10 bg-accent/15"
                    : "border-white/10 bg-(--color-surface-elevated)")
                }
              >
                <div
                  className={
                    "text-[11px] font-bold uppercase tracking-wide " +
                    (day.isToday ? "text-accent" : "text-white/50")
                  }
                >
                  {day.dow}
                </div>
                <div className="mt-0.5 text-lg font-bold">{day.dnum}</div>
              </div>
              <div className="flex flex-col gap-2 p-2">
                {day.episodes.length > 0 ? (
                  day.episodes.map((ep) => <MiniCard key={ep.episodeId} episode={ep} />)
                ) : (
                  <div className="py-4 text-center text-[11px] text-white/30">
                    —
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

/// A compact week-cell tile — small thumbnail + show + "SxEy". Links into the
/// show like every other calendar card (/watch/{showId}).
function MiniCard({ episode }: { episode: CalendarEpisode }) {
  const thumbPath =
    episode.posterPath ?? episode.stillPath ?? episode.backdropPath ?? undefined;
  const img = plexImage(thumbPath, 120, 120);
  return (
    <Link
      href={`/watch/${episode.showId}`}
      aria-label={`${episode.showTitle} — S${episode.seasonNumber} E${episode.episodeNumber}`}
      className="group flex gap-2 rounded-lg bg-(--color-surface-elevated) p-1.5 transition-colors hover:bg-white/10 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent"
    >
      <div className="aspect-square w-10 shrink-0 overflow-hidden rounded bg-black/40">
        {img && (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={img}
            alt=""
            width={120}
            height={120}
            loading="lazy"
            className="block h-full w-full object-cover"
          />
        )}
      </div>
      <div className="min-w-0 flex-1 py-0.5">
        <div className="line-clamp-2 text-[12px] font-semibold leading-tight">
          {episode.showTitle}
        </div>
        <div className="mt-0.5 text-[10.5px] text-white/50">
          S{episode.seasonNumber} · E{episode.episodeNumber}
        </div>
      </div>
    </Link>
  );
}
