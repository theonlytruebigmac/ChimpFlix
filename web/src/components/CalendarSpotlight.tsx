import { CalendarEpisodeCard } from "@/components/CalendarEpisodeCard";
import { CalendarEpisodeRow } from "@/components/CalendarEpisodeRow";
import {
  relativeDayLabel,
  airDateShort,
  calendarDayDelta,
} from "@/lib/relative-time";
import type { CalendarDayGroup } from "@/components/CalendarClient";

/// "Today Spotlight" calendar view — the page's headline layout. Today's
/// releases sit up top as large hero cards (or a friendly empty state when
/// nothing airs today), then "Up Next" lists the days ahead as a compact
/// agenda, and any already-aired look-back day trails as a dimmed "Recently
/// aired" group. Each day group is classified by its signed delta from the
/// viewer's local today (< 0 past, 0 today, > 0 upcoming).
export function CalendarSpotlight({
  groups,
  nowMs,
}: {
  groups: CalendarDayGroup[];
  nowMs: number;
}) {
  // Classify each day group once by its delta from the viewer's local today.
  // The page fetches a wide window (for the Week / Month views); the spotlight
  // intentionally shows only a focused slice of it: today, the next ~5 weeks
  // ("Up Next"), and just yesterday ("Recently aired").
  const classified = groups.map((g) => ({
    g,
    delta: calendarDayDelta(g.labelMs, nowMs),
  }));
  const today = classified.find((c) => c.delta === 0)?.g ?? null;
  const upcoming = classified
    .filter((c) => c.delta > 0 && c.delta <= 35)
    .map((c) => c.g);
  const recent = classified.filter((c) => c.delta === -1).map((c) => c.g);

  // The viewer's *current* local date for the Today heading — a real instant,
  // so it's read in local time (unlike air dates, which are UTC plain dates).
  const todayLabel = new Date(nowMs).toLocaleDateString(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
  });

  return (
    <div className="space-y-12">
      {/* Today spotlight */}
      <section>
        <div className="mb-4 flex items-baseline gap-3">
          <h2 className="text-2xl font-bold tracking-tight">Today</h2>
          <span className="text-sm text-white/55">{todayLabel}</span>
        </div>
        {today ? (
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {today.episodes.map((ep) => (
              <CalendarEpisodeCard
                key={ep.episodeId}
                episode={ep}
                nowMs={nowMs}
                showWhen={false}
                className="shadow-lg shadow-black/40"
              />
            ))}
          </div>
        ) : (
          <div className="rounded-xl border border-dashed border-white/15 px-6 py-10 text-center">
            <div className="text-base font-semibold text-white/80">
              Nothing airing today
            </div>
            <p className="mt-1 text-sm text-white/50">
              {upcoming.length > 0
                ? `Next up ${relativeDayLabel(upcoming[0].labelMs, nowMs)} — ${upcoming[0].episodes[0].showTitle}.`
                : "New episodes from your shows will appear here on their air date."}
            </p>
          </div>
        )}
      </section>

      {/* Up Next */}
      {upcoming.length > 0 && (
        <AgendaSection title="Up Next" groups={upcoming} nowMs={nowMs} />
      )}

      {/* Recently aired (the look-back day) — de-emphasized */}
      {recent.length > 0 && (
        <AgendaSection
          title="Recently Aired"
          groups={recent}
          nowMs={nowMs}
          dim
        />
      )}
    </div>
  );
}

/// A titled list of day groups, each a sticky relative-day heading on the left
/// and its episode rows on the right.
function AgendaSection({
  title,
  groups,
  nowMs,
  dim = false,
}: {
  title: string;
  groups: CalendarDayGroup[];
  nowMs: number;
  dim?: boolean;
}) {
  return (
    <section className={dim ? "opacity-70" : undefined}>
      <h3 className="mb-4 text-xs font-bold uppercase tracking-[0.12em] text-white/45">
        {title}
      </h3>
      <div className="space-y-5">
        {groups.map((g) => {
          const label = relativeDayLabel(g.labelMs, nowMs);
          // The "Weekday, Mon D" form already carries the date; the bare-word
          // forms (Today / Tomorrow / Friday) get a secondary date line.
          const sub = label.includes(",") ? null : airDateShort(g.labelMs);
          return (
            <div
              key={g.key}
              className="grid grid-cols-1 gap-2 sm:grid-cols-[120px_1fr] sm:gap-5"
            >
              <div className="sm:sticky sm:top-20 sm:self-start">
                <div className="text-sm font-bold">{label}</div>
                {sub && (
                  <div className="text-xs text-white/40">{sub}</div>
                )}
              </div>
              <div className="flex flex-col gap-1.5">
                {g.episodes.map((ep) => (
                  <CalendarEpisodeRow key={ep.episodeId} episode={ep} />
                ))}
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}
