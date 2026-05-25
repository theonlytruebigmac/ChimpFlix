"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import Link from "next/link";
import {
  admin as adminApi,
  type NowPlayingSession,
  type StatsActivityRow,
  type StatsDailyBucket,
  type StatsHourBucket,
  type StatsLibraryBucket,
  type StatsOverview,
  type StatsPlatformBucket,
  type StatsTopItemRow,
  type StatsTopUserRow,
} from "@/lib/chimpflix-api";
import { formatDate } from "@/lib/format";
import { LoadingPlaceholder } from "../ui/LoadingPlaceholder";

// Frontend-only concurrent-streams sparkline — keeps the last
// CONCURRENT_HISTORY samples from the now-playing poll and draws
// them as a tiny sparkline. Server-side historical sampling would
// need a separate samples table; this rolling client-side window
// covers the common "how spiky was the last few minutes" need
// without any backend addition.
const CONCURRENT_HISTORY = 60; // 60 samples × 5s poll = 5 min window

const WINDOW_OPTIONS = [7, 14, 30, 90, 365] as const;
const NOW_PLAYING_POLL_MS = 5_000;

interface OverviewResponse {
  days: number;
  overview: StatsOverview;
  now_playing_count: number;
}

interface Props {
  initialOverview: OverviewResponse;
  initialActivity: StatsActivityRow[];
  initialTopUsers: StatsTopUserRow[];
  initialTopItems: StatsTopItemRow[];
  initialTopPlatforms: StatsPlatformBucket[];
  initialTopLibraries: StatsLibraryBucket[];
  initialNowPlaying: NowPlayingSession[];
  initialPerDay: StatsDailyBucket[];
  initialPerHour: StatsHourBucket[];
}

export function AdminStatsClient({
  initialOverview,
  initialActivity,
  initialTopUsers,
  initialTopItems,
  initialTopPlatforms,
  initialTopLibraries,
  initialNowPlaying,
  initialPerDay,
  initialPerHour,
}: Props) {
  const [days, setDays] = useState<number>(initialOverview.days);
  const [overview, setOverview] = useState(initialOverview);
  const [activity, setActivity] = useState(initialActivity);
  const [topUsers, setTopUsers] = useState(initialTopUsers);
  const [topItems, setTopItems] = useState(initialTopItems);
  const [topPlatforms, setTopPlatforms] = useState(initialTopPlatforms);
  const [topLibraries, setTopLibraries] = useState(initialTopLibraries);
  const [nowPlaying, setNowPlaying] = useState(initialNowPlaying);
  const [perDay, setPerDay] = useState(initialPerDay);
  const [perHour, setPerHour] = useState(initialPerHour);
  // Rolling concurrent-streams series — pushed on every now-playing
  // poll, capped at CONCURRENT_HISTORY samples. Seeded with the
  // current count so the sparkline isn't a flat zero on first paint.
  const [concurrentHistory, setConcurrentHistory] = useState<number[]>(() =>
    Array(CONCURRENT_HISTORY).fill(initialNowPlaying.length),
  );
  const [error, setError] = useState<string | null>(null);
  /// When set, opens the per-user drill-in modal. Tuple keeps the
  /// display name handy for the modal title without a second lookup.
  const [drillUser, setDrillUser] = useState<{ id: number; label: string } | null>(
    null,
  );

  // Refetch every windowed section whenever the operator changes the
  // time window. now-playing is live and not gated by `days`; activity
  // is unscoped (the feed shows the latest N regardless of window).
  useEffect(() => {
    if (days === initialOverview.days) return; // first render — already SSR'd
    let cancelled = false;
    (async () => {
      try {
        const [ov, tu, ti, tp, tl, pd, ph] = await Promise.all([
          adminApi.stats.overview(days),
          adminApi.stats.topUsers({ days, limit: 10 }),
          adminApi.stats.topItems({ days, limit: 10 }),
          adminApi.stats.topPlatforms({ days, limit: 8 }),
          adminApi.stats.topLibraries({ days, limit: 10 }),
          adminApi.stats.playsPerDay(days),
          adminApi.stats.playsPerHour(days),
        ]);
        if (cancelled) return;
        setOverview(ov);
        setTopUsers(tu.users);
        setTopItems(ti.items);
        setTopPlatforms(tp.platforms);
        setTopLibraries(tl.libraries);
        setPerDay(pd.buckets);
        setPerHour(ph.buckets);
        setError(null);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [days, initialOverview.days]);

  // Poll now-playing on a short interval. Cheap (memory-backed snapshot
  // on the server), and the operator usually arrives here exactly when
  // they want to see the live picture. Each sample also gets pushed
  // into the rolling concurrent-streams series the sparkline reads.
  useEffect(() => {
    const tick = async () => {
      try {
        const r = await adminApi.stats.nowPlaying();
        setNowPlaying(r.sessions);
        setConcurrentHistory((h) => {
          const next = [...h, r.sessions.length];
          return next.slice(-CONCURRENT_HISTORY);
        });
      } catch {
        // Soft-fail; static tiles still useful even if polling drops.
      }
    };
    const id = window.setInterval(tick, NOW_PLAYING_POLL_MS);
    return () => window.clearInterval(id);
  }, []);

  const refreshActivity = useCallback(async () => {
    try {
      const r = await adminApi.stats.activity({ limit: 50 });
      setActivity(r.events);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  const transcodePct = useMemo(() => {
    const total = overview.overview.direct_plays + overview.overview.transcoded_plays;
    if (total === 0) return null;
    return Math.round((overview.overview.transcoded_plays / total) * 100);
  }, [overview]);

  return (
    <div className="space-y-6">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      {/* ─── Window picker ─── */}
      <div className="flex items-center gap-2 text-sm">
        <span className="text-white/55">Window:</span>
        {WINDOW_OPTIONS.map((d) => (
          <button
            key={d}
            type="button"
            onClick={() => setDays(d)}
            className={`rounded px-2.5 py-1 text-xs font-medium transition ${
              days === d
                ? "bg-(--color-accent) text-white"
                : "bg-white/5 text-white/70 hover:bg-white/10"
            }`}
          >
            {d === 365 ? "Last year" : `Last ${d}d`}
          </button>
        ))}
      </div>

      {/* ─── Hero tiles ─── */}
      <section className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <Tile
          label="Total plays"
          value={overview.overview.total_plays.toLocaleString()}
          sub={`${overview.overview.completions.toLocaleString()} completions`}
        />
        <Tile
          label="Unique users"
          value={overview.overview.unique_users.toLocaleString()}
          sub={`Active in last ${days}d`}
        />
        <Tile
          label="Transcode mix"
          value={transcodePct == null ? "—" : `${transcodePct}%`}
          sub={
            transcodePct == null
              ? "No streams yet"
              : `${overview.overview.direct_plays} direct · ${overview.overview.transcoded_plays} transcoded`
          }
        />
        <Tile
          label="Now playing"
          value={nowPlaying.length.toLocaleString()}
          sub={
            nowPlaying.length > 0
              ? `Peak in last 5m: ${Math.max(...concurrentHistory)}`
              : "No active sessions"
          }
          highlight={nowPlaying.length > 0}
          accessory={
            <ConcurrentSparkline samples={concurrentHistory} />
          }
        />
      </section>

      {/* ─── Now playing ─── */}
      <Section title="Now playing" hint="Live transcode sessions from the in-memory session manager. Polled every 5 seconds.">
        {nowPlaying.length === 0 ? (
          <EmptyState>No active sessions.</EmptyState>
        ) : (
          <ul className="divide-y divide-white/5">
            {nowPlaying.map((s) => (
              <li key={s.id} className="flex flex-wrap items-baseline justify-between gap-3 py-2 text-sm">
                <div>
                  <div className="font-medium">
                    User #{s.user_id} · {s.encoder}
                  </div>
                  <div className="text-xs text-white/55">
                    {describeTreatment(s)} · started {formatRelative(s.created_at)}
                  </div>
                </div>
                <div className="text-right text-xs text-white/55">
                  <div>{formatBytes(s.bytes_served)} served</div>
                  <div>Last seen {formatRelative(s.last_seen_at)}</div>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Section>

      {/* ─── Time-series charts ─── */}
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <Section
          title={`Plays per day · last ${days}d`}
          hint="Stacked bars: starts on bottom, completions on top. Hover a bar for the exact date + counts."
        >
          {perDay.every((b) => b.starts === 0 && b.completions === 0) ? (
            <EmptyState>No events in the selected window.</EmptyState>
          ) : (
            <DailyChart buckets={perDay} />
          )}
        </Section>
        <Section
          title={`Plays by hour of day · last ${days}d`}
          hint="When your household actually watches. Aligned to server local time."
        >
          {perHour.every((b) => b.starts === 0) ? (
            <EmptyState>No events in the selected window.</EmptyState>
          ) : (
            <HourChart buckets={perHour} />
          )}
        </Section>
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        {/* ─── Top users (clickable → per-user drill-in) ─── */}
        <Section title={`Top users · last ${days}d`} hint="Click a row to see their history.">
          {topUsers.length === 0 ? (
            <EmptyState>No plays in the selected window.</EmptyState>
          ) : (
            <ol className="divide-y divide-white/5">
              {topUsers.map((u, i) => (
                <li key={u.user_id}>
                  <button
                    type="button"
                    onClick={() =>
                      setDrillUser({
                        id: u.user_id,
                        label: u.display_name ?? u.username,
                      })
                    }
                    className="flex w-full items-baseline justify-between gap-3 py-2 text-left text-sm transition-colors hover:bg-white/2"
                  >
                    <div className="flex items-baseline gap-3">
                      <span className="w-5 text-right text-xs text-white/40">
                        {i + 1}
                      </span>
                      <div>
                        <div className="font-medium">
                          {u.display_name ?? u.username}
                        </div>
                        <div className="text-xs text-white/55">
                          @{u.username} ·{" "}
                          {u.last_seen_at
                            ? `last seen ${formatRelative(u.last_seen_at)}`
                            : "no recent activity"}
                        </div>
                      </div>
                    </div>
                    <div className="text-right text-xs">
                      <div className="font-semibold text-white">
                        {u.play_count} {u.play_count === 1 ? "play" : "plays"}
                      </div>
                      <div className="text-white/55">
                        {u.completions} completed
                      </div>
                    </div>
                  </button>
                </li>
              ))}
            </ol>
          )}
        </Section>

        {/* ─── Top items ─── */}
        <Section title={`Top titles · last ${days}d`}>
          {topItems.length === 0 ? (
            <EmptyState>No plays in the selected window.</EmptyState>
          ) : (
            <ol className="divide-y divide-white/5">
              {topItems.map((it, i) => (
                <li
                  key={it.item_id ?? `${it.title}-${i}`}
                  className="flex items-baseline justify-between gap-3 py-2 text-sm"
                >
                  <div className="flex items-baseline gap-3">
                    <span className="w-5 text-right text-xs text-white/40">
                      {i + 1}
                    </span>
                    <div>
                      <div className="font-medium">{it.title}</div>
                      <div className="text-xs text-white/55">
                        {it.kind === "show" ? "Series" : "Film"}
                        {it.year ? ` · ${it.year}` : ""}
                        {it.last_played_at
                          ? ` · last played ${formatRelative(it.last_played_at)}`
                          : ""}
                      </div>
                    </div>
                  </div>
                  <div className="text-right text-xs font-semibold">
                    {it.play_count} {it.play_count === 1 ? "play" : "plays"}
                  </div>
                </li>
              ))}
            </ol>
          )}
        </Section>

        {/* ─── Top platforms ─── */}
        <Section
          title={`Top platforms · last ${days}d`}
          hint="Coarse bucket from the user-agent string — Chrome, Firefox, iOS, LG TV, Roku, etc."
        >
          {topPlatforms.length === 0 ? (
            <EmptyState>No platform data in the selected window.</EmptyState>
          ) : (
            <ol className="space-y-2">
              {topPlatforms.map((p) => (
                <PlatformBar
                  key={p.platform}
                  label={p.platform}
                  starts={p.starts}
                  max={Math.max(...topPlatforms.map((x) => x.starts))}
                />
              ))}
            </ol>
          )}
        </Section>
      </div>

      {/* ─── Top libraries ─── */}
      <Section
        title={`Top libraries · last ${days}d`}
        hint="Which libraries get the most play activity. Episodes roll up to their parent show's library."
      >
        {topLibraries.length === 0 ? (
          <EmptyState>No library activity in the selected window.</EmptyState>
        ) : (
          <ol className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {topLibraries.map((l) => (
              <PlatformBar
                key={l.library_id}
                label={`${l.name} (${l.kind === "movies" ? "Movies" : l.kind === "shows" ? "TV" : "Anime"})`}
                starts={l.starts}
                max={Math.max(...topLibraries.map((x) => x.starts))}
              />
            ))}
          </ol>
        )}
      </Section>

      {/* ─── Recent activity feed ─── */}
      <Section
        title="Recent activity"
        hint="Most recent 50 events. Refreshes on demand."
        action={
          <button
            type="button"
            onClick={refreshActivity}
            className="rounded border border-white/15 px-2.5 py-1 text-xs text-white/70 hover:bg-white/5"
          >
            Refresh
          </button>
        }
      >
        {activity.length === 0 ? (
          <EmptyState>No events recorded yet. Hit Play on a title.</EmptyState>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-sm">
              <thead className="text-xs uppercase tracking-wider text-white/40">
                <tr>
                  <th className="px-3 py-2">When</th>
                  <th className="px-3 py-2">User</th>
                  <th className="px-3 py-2">Event</th>
                  <th className="px-3 py-2">Title</th>
                  <th className="px-3 py-2">Decision</th>
                  <th className="px-3 py-2">IP</th>
                </tr>
              </thead>
              <tbody>
                {activity.map((e) => (
                  <tr key={e.id} className="border-t border-white/5">
                    <td className="whitespace-nowrap px-3 py-2 text-xs text-white/55">
                      {formatRelative(e.occurred_at)}
                    </td>
                    <td className="whitespace-nowrap px-3 py-2">@{e.username}</td>
                    <td className="px-3 py-2">
                      <EventPill type={e.event_type} />
                    </td>
                    <td className="px-3 py-2">
                      {e.item_id ? (
                        <Link
                          href={`/?title=${e.item_id}`}
                          className="text-white hover:underline"
                        >
                          {e.title ?? `Item #${e.item_id}`}
                        </Link>
                      ) : (
                        <span className="text-white/70">
                          {e.title ?? (e.episode_id ? `Episode #${e.episode_id}` : "—")}
                        </span>
                      )}
                    </td>
                    <td className="px-3 py-2 text-xs">
                      {e.decision ? <DecisionPill decision={e.decision} /> : "—"}
                    </td>
                    <td className="px-3 py-2 text-xs text-white/55">
                      {e.ip ?? "unknown"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Section>

      {drillUser && (
        <UserDrillIn
          userId={drillUser.id}
          label={drillUser.label}
          onClose={() => setDrillUser(null)}
        />
      )}
    </div>
  );
}

// ─── Charts ───────────────────────────────────────────────────────────────
//
// Inline SVG — keeps the dep tree clean and the charts are simple enough
// that hand-rolled bars + a sparkline cover the use case. If we add more
// chart shapes (line, pie) later, a small wrapper crate becomes
// worthwhile; for now two functions are cheaper than the dep.

function DailyChart({ buckets }: { buckets: StatsDailyBucket[] }) {
  const width = 100;
  const height = 36;
  const max = Math.max(
    1,
    ...buckets.map((b) => b.starts + b.completions),
  );
  const slot = width / Math.max(1, buckets.length);
  const barWidth = Math.max(0.5, slot * 0.78);
  const gap = slot - barWidth;
  // Render bars as a viewBox-relative path so we don't have to hardcode
  // pixel widths; the SVG fills its container.
  return (
    <div className="space-y-2">
      <svg
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="none"
        className="block h-32 w-full"
        role="img"
        aria-label="Daily plays"
      >
        {buckets.map((b, i) => {
          const x = i * slot + gap / 2;
          const total = b.starts + b.completions;
          const totalH = (total / max) * (height - 2);
          const startH = (b.starts / max) * (height - 2);
          const completeH = (b.completions / max) * (height - 2);
          const yTotal = height - totalH;
          return (
            <g key={b.day}>
              {/* Completions sit ON TOP of starts. Render starts first
                  (lower band) then completions stacked above. */}
              <rect
                x={x}
                y={height - startH}
                width={barWidth}
                height={startH}
                fill="rgba(229, 9, 20, 0.85)"
              />
              <rect
                x={x}
                y={yTotal}
                width={barWidth}
                height={completeH}
                fill="rgba(96, 165, 250, 0.85)"
              />
              {/* Wide invisible hit target for tooltip on hover. */}
              <rect x={x} y={0} width={barWidth} height={height} fill="transparent">
                <title>
                  {b.day} — {b.starts} start{b.starts === 1 ? "" : "s"},{" "}
                  {b.completions} complete{b.completions === 1 ? "" : "s"}
                </title>
              </rect>
            </g>
          );
        })}
      </svg>
      <ChartLegend
        entries={[
          { label: "Starts", color: "rgba(229, 9, 20, 0.85)" },
          { label: "Completions", color: "rgba(96, 165, 250, 0.85)" },
        ]}
      />
    </div>
  );
}

function HourChart({ buckets }: { buckets: StatsHourBucket[] }) {
  const width = 100;
  const height = 36;
  const max = Math.max(1, ...buckets.map((b) => b.starts));
  const slot = width / 24;
  const barWidth = slot * 0.7;
  return (
    <div className="space-y-1">
      <svg
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="none"
        className="block h-32 w-full"
        role="img"
        aria-label="Plays by hour of day"
      >
        {buckets.map((b) => {
          const x = b.hour * slot + (slot - barWidth) / 2;
          const h = (b.starts / max) * (height - 2);
          return (
            <g key={b.hour}>
              <rect
                x={x}
                y={height - h}
                width={barWidth}
                height={h}
                fill="rgba(229, 9, 20, 0.85)"
              />
              <rect x={x} y={0} width={barWidth} height={height} fill="transparent">
                <title>
                  {String(b.hour).padStart(2, "0")}:00 — {b.starts} start
                  {b.starts === 1 ? "" : "s"}
                </title>
              </rect>
            </g>
          );
        })}
      </svg>
      {/* X-axis ticks every 6 hours so the chart stays readable at any
          width without crowding. */}
      <div className="flex justify-between px-0.5 text-[10px] text-white/40">
        {[0, 6, 12, 18, 23].map((h) => (
          <span key={h}>{String(h).padStart(2, "0")}:00</span>
        ))}
      </div>
    </div>
  );
}

function ChartLegend({
  entries,
}: {
  entries: Array<{ label: string; color: string }>;
}) {
  return (
    <div className="flex flex-wrap gap-3 text-[10px] uppercase tracking-wider text-white/55">
      {entries.map((e) => (
        <span key={e.label} className="inline-flex items-center gap-1.5">
          <span
            className="inline-block h-2 w-2 rounded-sm"
            style={{ background: e.color }}
            aria-hidden
          />
          {e.label}
        </span>
      ))}
    </div>
  );
}

function PlatformBar({
  label,
  starts,
  max,
}: {
  /// Used by both the platforms list (browser/OS name) and the
  /// libraries list (library name + kind). Same visual; rename so
  /// neither caller looks weird.
  label: string;
  starts: number;
  max: number;
}) {
  const pct = max === 0 ? 0 : Math.round((starts / max) * 100);
  return (
    <li className="space-y-1">
      <div className="flex items-baseline justify-between text-xs">
        <span className="font-medium text-white/90">{label}</span>
        <span className="text-white/55">{starts}</span>
      </div>
      <div className="h-1.5 overflow-hidden rounded bg-white/5">
        <div
          className="h-full rounded bg-accent"
          style={{ width: `${pct}%` }}
        />
      </div>
    </li>
  );
}

/// Human-readable byte formatter: 1.4 GB / 23.5 MB / 512 KB / 124 B.
/// Used by the Now Playing rows for `bytes_served`. Tiny utility so
/// it lives here next to its only caller.
function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`;
}

// ─── Per-user drill-in modal ─────────────────────────────────────────────
//
// Loads that user's most-recent N activity events on open. Portal'd to
// document.body so it's truly viewport-centered (the modal-card
// containing-block trick from elsewhere in the app doesn't apply here,
// since we render at the page root anyway — but portaling future-proofs
// against a parent ever growing a transform).

function UserDrillIn({
  userId,
  label,
  onClose,
}: {
  userId: number;
  label: string;
  onClose: () => void;
}) {
  const [events, setEvents] = useState<StatsActivityRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const r = await adminApi.stats.activity({ limit: 100, user_id: userId });
        if (!cancelled) setEvents(r.events);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [userId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  if (typeof document === "undefined") return null;
  return createPortal(
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-60 flex items-center justify-center bg-black/70 p-4"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="w-full max-w-3xl overflow-hidden rounded-lg border border-white/15 bg-(--color-surface) shadow-2xl">
        <div className="flex items-baseline justify-between border-b border-white/10 px-6 py-4">
          <div>
            <h2 className="text-lg font-semibold">{label}</h2>
            <p className="text-xs text-white/55">Most recent 100 events</p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="text-white/60 transition-colors hover:text-white"
          >
            ✕
          </button>
        </div>
        <div className="max-h-[70vh] overflow-y-auto">
          {error && (
            <div className="px-6 py-3 text-xs text-red-300">{error}</div>
          )}
          {events == null ? (
            <LoadingPlaceholder />
          ) : events.length === 0 ? (
            <div className="px-6 py-8 text-center text-sm text-white/55">
              No events recorded for this user yet.
            </div>
          ) : (
            <table className="w-full text-left text-sm">
              <thead className="text-xs uppercase tracking-wider text-white/40">
                <tr>
                  <th className="px-6 py-2">When</th>
                  <th className="px-6 py-2">Event</th>
                  <th className="px-6 py-2">Title</th>
                  <th className="px-6 py-2">Decision</th>
                  <th className="px-6 py-2">IP</th>
                </tr>
              </thead>
              <tbody>
                {events.map((e) => (
                  <tr key={e.id} className="border-t border-white/5">
                    <td className="whitespace-nowrap px-6 py-2 text-xs text-white/55">
                      {formatRelative(e.occurred_at)}
                    </td>
                    <td className="px-6 py-2">
                      <EventPill type={e.event_type} />
                    </td>
                    <td className="px-6 py-2">
                      {e.title ?? (e.episode_id
                        ? `Episode #${e.episode_id}`
                        : e.item_id
                          ? `Item #${e.item_id}`
                          : "—")}
                    </td>
                    <td className="px-6 py-2 text-xs">
                      {e.decision ? <DecisionPill decision={e.decision} /> : "—"}
                    </td>
                    <td className="px-6 py-2 text-xs text-white/55">
                      {e.ip ?? "unknown"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>
    </div>,
    document.body,
  );
}

// ─── Small building blocks ────────────────────────────────────────────────

function Tile({
  label,
  value,
  sub,
  highlight,
  accessory,
}: {
  label: string;
  value: string;
  sub: string;
  highlight?: boolean;
  /// Optional element rendered to the right of value — used by the
  /// Now Playing tile to inline a concurrent-streams sparkline.
  accessory?: React.ReactNode;
}) {
  return (
    <div
      className={`rounded-lg border p-4 ${
        highlight
          ? "border-accent/40 bg-accent/10"
          : "border-white/10 bg-white/2"
      }`}
    >
      <div className="text-xs uppercase tracking-wider text-white/55">{label}</div>
      <div className="mt-1 flex items-end justify-between gap-3">
        <div className="text-3xl font-bold tabular-nums">{value}</div>
        {accessory}
      </div>
      <div className="mt-1 text-xs text-white/55">{sub}</div>
    </div>
  );
}

function ConcurrentSparkline({ samples }: { samples: number[] }) {
  // SVG sparkline. Empty / all-zero series collapses to a flat
  // baseline so the tile doesn't render an invisible chart.
  const max = Math.max(1, ...samples);
  const w = 60;
  const h = 18;
  const step = samples.length > 1 ? w / (samples.length - 1) : w;
  const points = samples
    .map((v, i) => {
      const x = i * step;
      const y = h - (v / max) * (h - 2) - 1;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg
      viewBox={`0 0 ${w} ${h}`}
      preserveAspectRatio="none"
      className="h-5 w-16 text-white/45"
      role="img"
      aria-label="Concurrent streams (last 5 minutes)"
    >
      <polyline
        fill="none"
        stroke="currentColor"
        strokeWidth="1"
        points={points}
      />
    </svg>
  );
}

function Section({
  title,
  hint,
  action,
  children,
}: {
  title: string;
  hint?: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-lg border border-white/10 bg-white/2 p-5">
      <div className="mb-3 flex items-baseline justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold">{title}</h2>
          {hint && <p className="mt-0.5 text-xs text-white/45">{hint}</p>}
        </div>
        {action}
      </div>
      {children}
    </section>
  );
}

function EmptyState({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded border border-dashed border-white/10 bg-white/2 px-4 py-6 text-center text-xs text-white/45">
      {children}
    </div>
  );
}

function EventPill({ type }: { type: StatsActivityRow["event_type"] }) {
  const styles: Record<string, string> = {
    start: "bg-emerald-500/15 text-emerald-300",
    complete: "bg-blue-500/15 text-blue-300",
    pause: "bg-amber-500/15 text-amber-300",
    resume: "bg-emerald-500/15 text-emerald-300",
    stop: "bg-white/10 text-white/60",
    progress: "bg-white/10 text-white/60",
  };
  return (
    <span
      className={`rounded px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider ${
        styles[type] ?? "bg-white/10 text-white/60"
      }`}
    >
      {type}
    </span>
  );
}

function DecisionPill({ decision }: { decision: "direct" | "transcode" }) {
  return decision === "direct" ? (
    <span className="rounded bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider text-emerald-300">
      Direct
    </span>
  ) : (
    <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider text-amber-300">
      Transcode
    </span>
  );
}

function describeTreatment(s: NowPlayingSession): string {
  const v = s.video_treatment === "copy" ? "remux video" : "re-encode video";
  const a = s.audio_treatment === "copy" ? "remux audio" : "re-encode audio";
  const res = s.source_height
    ? `${s.source_height}p → ${s.target_height}p`
    : `${s.target_height}p`;
  return `${res} · ${v} · ${a}`;
}

// Short "5s ago", "12m ago", "3d ago" — mirror of the standard pattern
// used elsewhere in the admin shell (alerts feed, sessions list).
function formatRelative(epochMs: number): string {
  const delta = Math.max(0, Date.now() - epochMs);
  const s = Math.floor(delta / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.floor(h / 24);
  if (d < 30) return `${d}d ago`;
  return formatDate(epochMs);
}
