"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { airDayKey } from "@/lib/relative-time";
import type { CalendarEpisode } from "@/lib/chimpflix-api";

const DAY_MS = 86_400_000;
/// Max episode chips per day cell before collapsing the rest into "+N more".
const MAX_PER_DAY = 3;

/// "Month" calendar view — a full month grid (Sun–Sat) you can page through,
/// the long-horizon companion to the Week board. Leading/trailing days from the
/// adjacent months are dimmed, today's cell is outlined.
///
/// Like the week board, all day math runs in UTC-midnight "anchor" space —
/// `air_date` is a midnight-UTC plain date, so cells anchor to `Date.UTC(...)`
/// and match episodes by `airDayKey` (also UTC). "Today" is the viewer's LOCAL
/// date projected into that anchor space, so the highlight tracks the viewer's
/// calendar day; working in UTC anchors also sidesteps DST.
export function CalendarMonthGrid({
  episodes,
  nowMs,
}: {
  episodes: CalendarEpisode[];
  nowMs: number;
}) {
  const [monthOffset, setMonthOffset] = useState(0);

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

  const n = new Date(nowMs);
  const todayAnchor = Date.UTC(n.getFullYear(), n.getMonth(), n.getDate());

  // First of the displayed month (Date.UTC normalizes month over/underflow, so
  // offsets across year boundaries just work).
  const first = new Date(Date.UTC(n.getFullYear(), n.getMonth() + monthOffset, 1));
  const dispYear = first.getUTCFullYear();
  const dispMonth = first.getUTCMonth();
  const firstDow = first.getUTCDay(); // 0 = Sunday
  const gridStart = first.getTime() - firstDow * DAY_MS;
  const daysInMonth = new Date(Date.UTC(dispYear, dispMonth + 1, 0)).getUTCDate();
  const rows = Math.ceil((firstDow + daysInMonth) / 7);

  const weekdayLabels = Array.from({ length: 7 }, (_, i) =>
    new Date(gridStart + i * DAY_MS).toLocaleDateString(undefined, {
      weekday: "short",
      timeZone: "UTC",
    }),
  );

  const cells = Array.from({ length: rows * 7 }, (_, i) => {
    const anchor = gridStart + i * DAY_MS;
    const d = new Date(anchor);
    return {
      anchor,
      dnum: d.getUTCDate(),
      inMonth: d.getUTCMonth() === dispMonth && d.getUTCFullYear() === dispYear,
      isToday: anchor === todayAnchor,
      episodes: byDay.get(airDayKey(anchor)) ?? [],
    };
  });

  const monthLabel = first.toLocaleDateString(undefined, {
    month: "long",
    year: "numeric",
    timeZone: "UTC",
  });

  return (
    <div>
      <div className="mb-4 flex items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <button
            type="button"
            aria-label="Previous month"
            onClick={() => setMonthOffset((o) => o - 1)}
            className="flex h-9 w-9 items-center justify-center rounded-lg border border-white/10 bg-(--color-surface) text-lg leading-none text-white/80 transition-colors hover:bg-(--color-surface-elevated) focus:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          >
            ‹
          </button>
          <span className="min-w-40 text-center text-base font-semibold">
            {monthLabel}
          </span>
          <button
            type="button"
            aria-label="Next month"
            onClick={() => setMonthOffset((o) => o + 1)}
            className="flex h-9 w-9 items-center justify-center rounded-lg border border-white/10 bg-(--color-surface) text-lg leading-none text-white/80 transition-colors hover:bg-(--color-surface-elevated) focus:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          >
            ›
          </button>
        </div>
        {monthOffset !== 0 && (
          <button
            type="button"
            onClick={() => setMonthOffset(0)}
            className="text-sm font-medium text-white/60 transition-colors hover:text-accent focus:outline-none focus-visible:text-accent"
          >
            Today
          </button>
        )}
      </div>

      {/* Horizontal scroll on narrow screens so the 7 columns stay legible. */}
      <div className="overflow-x-auto overscroll-x-contain pb-2">
        <div className="min-w-[760px]">
          <div className="mb-2 grid grid-cols-7 gap-2.5">
            {weekdayLabels.map((w) => (
              <div
                key={w}
                className="text-center text-[11px] font-bold uppercase tracking-wide text-white/40"
              >
                {w}
              </div>
            ))}
          </div>
          <div className="grid grid-cols-7 gap-2.5">
            {cells.map((cell) => {
              const extra = cell.episodes.length - MAX_PER_DAY;
              return (
                <div
                  key={cell.anchor}
                  className={
                    "flex min-h-28 flex-col rounded-lg border p-1.5 " +
                    (cell.isToday
                      ? "border-accent/50 ring-1 ring-accent/25 bg-accent/5"
                      : "border-white/10 bg-(--color-surface)") +
                    (cell.inMonth ? "" : " opacity-40")
                  }
                >
                  <div
                    className={
                      "mb-1 px-0.5 text-xs font-semibold " +
                      (cell.isToday ? "text-accent" : "text-white/55")
                    }
                  >
                    {cell.dnum}
                  </div>
                  <div className="flex flex-col gap-1">
                    {cell.episodes.slice(0, MAX_PER_DAY).map((ep) => (
                      <MonthEvent key={ep.episodeId} episode={ep} />
                    ))}
                    {extra > 0 && (
                      <div className="px-1 text-[10px] font-medium text-white/40">
                        +{extra} more
                      </div>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}

/// A single day-cell event chip — a thin accent-tinted bar with the show title
/// (truncated). Finale / premiere get a tinted variant. Links into the show
/// like every other calendar card (/watch/{showId}).
function MonthEvent({ episode }: { episode: CalendarEpisode }) {
  const tone = episode.isFinale
    ? "bg-[#ad2cff]/15 text-[#dcb6ff] hover:bg-[#ad2cff]/25"
    : episode.isPremiere
      ? "bg-accent/15 text-[#ffb0b0] hover:bg-accent/25"
      : "bg-white/8 text-white/80 hover:bg-white/15";
  return (
    <Link
      href={`/watch/${episode.showId}`}
      title={`${episode.showTitle} — S${episode.seasonNumber} E${episode.episodeNumber}`}
      className={
        "block truncate rounded-sm px-1.5 py-0.5 text-[10.5px] font-medium leading-tight transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-accent " +
        tone
      }
    >
      {episode.showTitle}
    </Link>
  );
}
