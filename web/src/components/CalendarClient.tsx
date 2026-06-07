"use client";

import { useEffect, useMemo, useState, type ReactNode } from "react";
import { CalendarSpotlight } from "@/components/CalendarSpotlight";
import { CalendarWeekBoard } from "@/components/CalendarWeekBoard";
import { CalendarMonthGrid } from "@/components/CalendarMonthGrid";
import { airDayKey } from "@/lib/relative-time";
import type { CalendarEpisode } from "@/lib/chimpflix-api";

/// A calendar day bucket — every episode airing on one calendar date. `air_date`
/// is a midnight-UTC plain date, so the bucket key is built in UTC (`airDayKey`),
/// independent of the viewer's clock. The today/upcoming/past split is computed
/// in the views against `nowMs` (see `calendarDayDelta`), since "today" depends
/// on the viewer's *current local date*.
export interface CalendarDayGroup {
  key: string;
  /// Representative air-date for the group (first episode's), for the heading.
  labelMs: number;
  episodes: CalendarEpisode[];
}

type View = "spotlight" | "week" | "month";

/// Calendar page body. The episode feed is fetched on the server (honoring the
/// auth cookie + per-library visibility) and grouped HERE by calendar day. Two
/// views share the grouped data: a Today-spotlight agenda (default) and a 7-day
/// week board.
///
/// `nowMs` is snapshotted *post-mount* (not during render): "Today" and the
/// relative-day labels depend on the browser's local date, so computing them
/// during SSR would both trip the react-hooks purity rule and risk an SSR/CSR
/// hydration mismatch across a timezone day boundary. Until the snapshot lands
/// we render a skeleton (the now-independent grouping is already done).
export function CalendarClient({ episodes }: { episodes: CalendarEpisode[] }) {
  const [view, setView] = useState<View>("spotlight");
  const [nowMs, setNowMs] = useState<number | null>(null);
  useEffect(() => {
    // Post-hydration clock read — deliberately client-only so SSR and the
    // first client render agree (avoids a timezone hydration mismatch). Same
    // pattern as SeasonEpisodes.tsx / Card.tsx.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setNowMs(Date.now());
  }, []);

  const groups = useMemo(() => {
    const byDay = new Map<string, CalendarDayGroup>();
    // The feed already arrives ordered by air_date asc, so the map's insertion
    // order is chronological — no re-sort needed.
    for (const ep of episodes) {
      const key = airDayKey(ep.airDate);
      const existing = byDay.get(key);
      if (existing) {
        existing.episodes.push(ep);
      } else {
        byDay.set(key, { key, labelMs: ep.airDate, episodes: [ep] });
      }
    }
    return [...byDay.values()];
  }, [episodes]);

  if (groups.length === 0) {
    return (
      <div className="rounded-lg border border-white/10 bg-(--color-surface) px-6 py-16 text-center">
        <div className="text-lg font-semibold">Nothing on the calendar yet</div>
        <p className="mx-auto mt-2 max-w-md text-sm text-white/55">
          Upcoming episodes from your shows will appear here once they have an
          air date. Check back after your next library scan.
        </p>
      </div>
    );
  }

  return (
    <div>
      <div className="mb-6 flex justify-end">
        <div
          role="tablist"
          aria-label="Calendar view"
          className="inline-flex gap-1 rounded-full border border-white/10 bg-(--color-surface) p-1"
        >
          <ViewTab
            active={view === "spotlight"}
            onClick={() => setView("spotlight")}
          >
            Spotlight
          </ViewTab>
          <ViewTab active={view === "week"} onClick={() => setView("week")}>
            Week
          </ViewTab>
          <ViewTab active={view === "month"} onClick={() => setView("month")}>
            Month
          </ViewTab>
        </div>
      </div>

      {nowMs === null ? (
        <CalendarSkeleton />
      ) : view === "spotlight" ? (
        <CalendarSpotlight groups={groups} nowMs={nowMs} />
      ) : view === "week" ? (
        <CalendarWeekBoard episodes={episodes} nowMs={nowMs} />
      ) : (
        <CalendarMonthGrid episodes={episodes} nowMs={nowMs} />
      )}
    </div>
  );
}

function ViewTab({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={onClick}
      className={
        "rounded-full px-4 py-1.5 text-sm font-semibold transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-accent " +
        (active
          ? "bg-(--color-surface-elevated) text-white"
          : "text-white/55 hover:text-white")
      }
    >
      {children}
    </button>
  );
}

/// Stable-layout placeholder shown until the post-mount `nowMs` snapshot lands
/// (one frame). Mirrors the spotlight's heading + hero-card grid so there's no
/// jump when the real content swaps in.
function CalendarSkeleton() {
  return (
    <div className="animate-pulse space-y-4" aria-hidden>
      <div className="h-7 w-40 rounded bg-white/10" />
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {[0, 1, 2].map((i) => (
          <div
            key={i}
            className="aspect-video w-full rounded-md bg-(--color-surface)"
          />
        ))}
      </div>
    </div>
  );
}
