"use client";

import { useMemo } from "react";
import { CalendarEpisodeCard } from "@/components/CalendarEpisodeCard";
import { relativeDayLabel, localDayKey } from "@/lib/relative-time";
import type { CalendarEpisode } from "@/lib/chimpflix-api";

/// Calendar page body. The episode feed is fetched on the server (so it
/// honors the auth cookie + per-library visibility) and handed down raw; the
/// day-grouping + relative-day headings happen HERE, on the client, because
/// they depend on the *browser's* local timezone — the backend stores
/// `airDate` at midnight UTC and we want to bucket each episode by the local
/// day it lands on for the viewer.
///
/// `nowMs` is snapshotted once in `useMemo` (not read during render on every
/// pass) so the grouping is stable across re-renders and every heading shares
/// one reference instant.
interface DayGroup {
  key: string;
  /// Representative air-date for the group (the first episode's), used to
  /// compute the heading label.
  labelMs: number;
  episodes: CalendarEpisode[];
}

export function CalendarClient({ episodes }: { episodes: CalendarEpisode[] }) {
  const { groups, nowMs } = useMemo(() => {
    const now = Date.now();
    const byDay = new Map<string, DayGroup>();
    // The feed already arrives ordered by air_date asc, so insertion order of
    // the map keys is chronological — no re-sort needed.
    for (const ep of episodes) {
      const key = localDayKey(ep.airDate);
      const existing = byDay.get(key);
      if (existing) {
        existing.episodes.push(ep);
      } else {
        byDay.set(key, { key, labelMs: ep.airDate, episodes: [ep] });
      }
    }
    return { groups: [...byDay.values()], nowMs: now };
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
    <div className="space-y-10">
      {groups.map((group) => (
        <section key={group.key}>
          <h2 className="mb-4 text-xl font-semibold tracking-tight">
            {relativeDayLabel(group.labelMs, nowMs)}
          </h2>
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
            {group.episodes.map((ep) => (
              <CalendarEpisodeCard
                key={ep.episodeId}
                episode={ep}
                nowMs={nowMs}
                showWhen={false}
              />
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}
