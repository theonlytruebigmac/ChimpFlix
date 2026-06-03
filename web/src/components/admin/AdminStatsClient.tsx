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

  const avgPerDay = useMemo(() => {
    if (days <= 0) return 0;
    return Math.round(overview.overview.total_plays / days);
  }, [overview, days]);

  const peakPerDay = useMemo(
    () => Math.max(0, ...perDay.map((b) => b.starts + b.completions)),
    [perDay],
  );

  return (
    <div>
      {/* Page title intentionally omitted — sidebar + breadcrumb name the page. */}

      {/* ── error banner ───────────────────────────────────────────── */}
      {error && (
        <div role="alert" aria-live="assertive" className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      {/* ── window picker ──────────────────────────────────────────── */}
      <div
        className="cf-flex cf-between"
        style={{ marginBottom: 16, flexWrap: "wrap", gap: 12 }}
      >
        <span className="cf-muted" style={{ fontSize: 12.5 }}>
          Playback trends across the server over the selected window.
        </span>
        <div className="cf-seg cf-accent">
          {WINDOW_OPTIONS.map((d) => (
            <button
              key={d}
              type="button"
              className={days === d ? "cf-on" : ""}
              onClick={() => setDays(d)}
            >
              {d === 365 ? "1y" : `${d}d`}
            </button>
          ))}
        </div>
      </div>

      {/* ── headline stats ─────────────────────────────────────────── */}
      <div className="cf-grid cf-c5">
        <Tile
          tone="cf-tone-red"
          label="Total plays"
          value={overview.overview.total_plays.toLocaleString()}
          meta={`${avgPerDay.toLocaleString()} plays/day avg`}
          icon={<polygon points="6 4 20 12 6 20 6 4" />}
        />
        <Tile
          tone="cf-tone-blue"
          label="Minutes watched"
          value={formatWatchMinutes(overview.overview.watched_ms)}
          meta={`≈ ${formatWatchHours(overview.overview.watched_ms).toLocaleString()} hours`}
          icon={
            <>
              <circle cx="12" cy="12" r="8" />
              <path d="M12 8v4l3 2" />
            </>
          }
        />
        <Tile
          tone="cf-tone-green"
          label="Unique users"
          value={overview.overview.unique_users.toLocaleString()}
          meta={`active in last ${days === 365 ? "year" : `${days}d`}`}
          icon={
            <>
              <circle cx="9" cy="8" r="3.5" />
              <path d="M3 20a6 6 0 0 1 12 0" />
              <path d="M16 5a3.5 3.5 0 0 1 0 7M21 20a6 6 0 0 0-5-5.9" />
            </>
          }
        />
        <Tile
          tone="cf-tone-violet"
          label="Transcodes"
          value={overview.overview.transcoded_plays.toLocaleString()}
          meta={
            transcodePct == null
              ? "no streams yet"
              : `${transcodePct}% of plays · ${overview.overview.direct_plays.toLocaleString()} direct`
          }
          icon={
            <>
              <rect x="6" y="6" width="12" height="12" rx="2" />
              <path d="M9 3v3M15 3v3M9 18v3M15 18v3M3 9h3M3 15h3M18 9h3M18 15h3" />
            </>
          }
        />
        <Tile
          tone="cf-tone-amber"
          label="Now playing"
          value={nowPlaying.length.toLocaleString()}
          meta={
            nowPlaying.length > 0
              ? `peak in last 5m: ${Math.max(...concurrentHistory)}`
              : "no active sessions"
          }
          icon={
            <>
              <path d="M5 8a8 8 0 0 0 0 8M8 5a12 12 0 0 0 0 14" />
              <path d="M19 8a8 8 0 0 1 0 8M16 5a12 12 0 0 1 0 14" />
              <circle cx="12" cy="12" r="1.5" />
            </>
          }
          accessory={<ConcurrentSparkline samples={concurrentHistory} />}
        />
      </div>

      {/* ── plays per day ──────────────────────────────────────────── */}
      <div className="cf-card" style={{ marginTop: 18 }}>
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Plays per day</div>
            <div className="cf-sub">
              Daily playback volume over the last {days === 365 ? "year" : `${days} days`}
            </div>
          </div>
          <div className="cf-head-aside">
            <span className="cf-pill cf-accent">
              <span className="cf-dot" />
              peak {peakPerDay.toLocaleString()}
            </span>
          </div>
        </div>
        <div className="cf-card-body cf-pad">
          {perDay.every((b) => b.starts === 0 && b.completions === 0) ? (
            <EmptyState>No events in the selected window.</EmptyState>
          ) : (
            <>
              <DailyChart buckets={perDay} />
              <div
                className="cf-flex cf-between cf-faint"
                style={{ fontSize: 11, marginTop: 8 }}
              >
                <span>
                  {days === 365 ? "1 year ago" : `${days} days ago`}
                </span>
                <span>
                  peak {peakPerDay.toLocaleString()} · avg{" "}
                  {avgPerDay.toLocaleString()}
                </span>
              </div>
            </>
          )}
        </div>
      </div>

      {/* ── plays by hour of day (production extra) ────────────────── */}
      <div className="cf-card" style={{ marginTop: 18 }}>
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Plays by hour of day</div>
            <div className="cf-sub">
              When your household actually watches. Aligned to server local time.
            </div>
          </div>
        </div>
        <div className="cf-card-body cf-pad">
          {perHour.every((b) => b.starts === 0) ? (
            <EmptyState>No events in the selected window.</EmptyState>
          ) : (
            <HourChart buckets={perHour} />
          )}
        </div>
      </div>

      {/* ── top titles + top users ─────────────────────────────────── */}
      <div className="cf-grid cf-c2" style={{ marginTop: 18, alignItems: "start" }}>
        <div className="cf-card" style={{ margin: 0 }}>
          <div className="cf-card-head">
            <div>
              <div className="cf-ttl">Top titles</div>
              <div className="cf-sub">Most-played this window</div>
            </div>
          </div>
          {topItems.length === 0 ? (
            <div className="cf-card-body cf-pad">
              <EmptyState>No plays in the selected window.</EmptyState>
            </div>
          ) : (
            <table className="cf-table">
              <tbody>
                {topItems.map((it, i) => (
                  <tr key={it.item_id ?? `${it.title}-${i}`}>
                    <td>
                      <div>{it.title}</div>
                      <div className="cf-faint" style={{ fontSize: 11.5, marginTop: 2 }}>
                        {it.kind === "show" ? "Series" : "Film"}
                        {it.year ? ` · ${it.year}` : ""}
                        {it.last_played_at
                          ? ` · last played ${formatRelative(it.last_played_at)}`
                          : ""}
                      </div>
                    </td>
                    <td className="cf-num">
                      {it.play_count.toLocaleString()}{" "}
                      {it.play_count === 1 ? "play" : "plays"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>

        <div className="cf-card" style={{ margin: 0 }}>
          <div className="cf-card-head">
            <div>
              <div className="cf-ttl">Top users</div>
              <div className="cf-sub">
                Watch time this window · click a row for history
              </div>
            </div>
          </div>
          {topUsers.length === 0 ? (
            <div className="cf-card-body cf-pad">
              <EmptyState>No plays in the selected window.</EmptyState>
            </div>
          ) : (
            <table className="cf-table">
              <tbody>
                {topUsers.map((u, i) => {
                  const name = u.display_name ?? u.username;
                  return (
                    <tr
                      key={u.user_id}
                      onClick={() => setDrillUser({ id: u.user_id, label: name })}
                      style={{ cursor: "pointer" }}
                    >
                      <td>
                        <div className="cf-flex cf-gap8">
                          <span
                            className={`cf-avatar ${avatarTone(i)}`}
                            style={{ width: 26, height: 26, fontSize: 10 }}
                          >
                            {initialOf(name)}
                          </span>
                          <div>
                            <div>{name}</div>
                            <div
                              className="cf-faint"
                              style={{ fontSize: 11.5, marginTop: 1 }}
                            >
                              @{u.username} ·{" "}
                              {u.last_seen_at
                                ? `last seen ${formatRelative(u.last_seen_at)}`
                                : "no recent activity"}
                            </div>
                          </div>
                        </div>
                      </td>
                      <td className="cf-num">
                        {formatWatchHours(u.watched_ms).toLocaleString()}h
                        <div
                          className="cf-faint"
                          style={{ fontSize: 11.5, marginTop: 1 }}
                        >
                          {u.play_count.toLocaleString()}{" "}
                          {u.play_count === 1 ? "play" : "plays"} ·{" "}
                          {u.completions.toLocaleString()} completed
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
      </div>

      {/* ── top platforms + top libraries ──────────────────────────── */}
      <div className="cf-grid cf-c2" style={{ marginTop: 18, alignItems: "start" }}>
        <div className="cf-card" style={{ margin: 0 }}>
          <div className="cf-card-head">
            <div>
              <div className="cf-ttl">Top platforms</div>
              <div className="cf-sub">Share of plays by client</div>
            </div>
          </div>
          {topPlatforms.length === 0 ? (
            <div className="cf-card-body cf-pad">
              <EmptyState>No platform data in the selected window.</EmptyState>
            </div>
          ) : (
            <table className="cf-table">
              <tbody>
                {(() => {
                  const platformTotal = topPlatforms.reduce((a, x) => a + x.starts, 0);
                  return topPlatforms.map((p) => {
                    const pct = platformTotal === 0 ? 0 : Math.round((p.starts / platformTotal) * 100);
                    return (
                      <BarRow key={p.platform} label={p.platform} pct={pct} />
                    );
                  });
                })()}
              </tbody>
            </table>
          )}
        </div>

        <div className="cf-card" style={{ margin: 0 }}>
          <div className="cf-card-head">
            <div>
              <div className="cf-ttl">Top libraries</div>
              <div className="cf-sub">Plays per library</div>
            </div>
          </div>
          {topLibraries.length === 0 ? (
            <div className="cf-card-body cf-pad">
              <EmptyState>No library activity in the selected window.</EmptyState>
            </div>
          ) : (
            <table className="cf-table">
              <tbody>
                {topLibraries.map((l) => (
                  <tr key={l.library_id}>
                    <td>
                      {l.name}
                      <span className="cf-faint" style={{ marginLeft: 6 }}>
                        (
                        {l.kind === "movies"
                          ? "Movies"
                          : l.kind === "shows"
                            ? "TV"
                            : "Anime"}
                        )
                      </span>
                    </td>
                    <td className="cf-num">
                      {l.starts.toLocaleString()}{" "}
                      {l.starts === 1 ? "play" : "plays"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>

      {/* ── now playing ────────────────────────────────────────────── */}
      <div className="cf-card" style={{ marginTop: 18 }}>
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Now playing</div>
            <div className="cf-sub">Live · refreshes every 5s</div>
          </div>
          <div className="cf-head-aside">
            <span className="cf-pill cf-accent">
              <span className="cf-dot" />
              {nowPlaying.length} {nowPlaying.length === 1 ? "stream" : "streams"}
            </span>
          </div>
        </div>
        {nowPlaying.length === 0 ? (
          <div className="cf-card-body cf-pad">
            <EmptyState>No active sessions.</EmptyState>
          </div>
        ) : (
          <table className="cf-table">
            <thead>
              <tr>
                <th>User</th>
                <th>Stream</th>
                <th className="cf-num">Resolution</th>
                <th className="cf-num">Served</th>
              </tr>
            </thead>
            <tbody>
              {nowPlaying.map((s, i) => (
                <tr key={s.id}>
                  <td>
                    <div className="cf-flex cf-gap8">
                      <span
                        className={`cf-avatar ${avatarTone(i)}`}
                        style={{ width: 28, height: 28, fontSize: 11 }}
                      >
                        {(s.username ?? `#${s.user_id}`).slice(0, 2).toUpperCase()}
                      </span>
                      <div>
                        <div>{s.username ?? `User #${s.user_id}`}</div>
                        <div
                          className="cf-faint"
                          style={{ fontSize: 11.5, marginTop: 1 }}
                        >
                          {s.title ?? `#${s.media_file_id}`}
                          {s.subtitle ? ` · ${s.subtitle}` : ""}
                        </div>
                      </div>
                    </div>
                  </td>
                  <td>
                    <TreatmentTag session={s} />
                    <div
                      className="cf-faint"
                      style={{ fontSize: 11.5, marginTop: 3 }}
                    >
                      {s.encoder} · started {formatRelative(s.created_at)} · last
                      seen {formatRelative(s.last_seen_at)}
                    </div>
                  </td>
                  <td className="cf-num cf-mono">
                    {s.source_height
                      ? `${s.source_height}p → ${s.target_height}p`
                      : `${s.target_height}p`}
                  </td>
                  <td className="cf-num cf-mono">{formatBytes(s.bytes_served)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      {/* ── recent activity feed (production extra) ────────────────── */}
      <div className="cf-card" style={{ marginTop: 18 }}>
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Recent activity</div>
            <div className="cf-sub">Most recent 50 events · refreshes on demand</div>
          </div>
          <div className="cf-head-aside">
            <button type="button" className="cf-btn cf-sm" onClick={refreshActivity}>
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M4 10a8 8 0 0 1 14-4l2 2M20 14a8 8 0 0 1-14 4l-2-2" />
                <path d="M18 4v4h-4M6 20v-4h4" />
              </svg>
              Refresh
            </button>
          </div>
        </div>
        {activity.length === 0 ? (
          <div className="cf-card-body cf-pad">
            <EmptyState>No events recorded yet. Hit Play on a title.</EmptyState>
          </div>
        ) : (
          <div style={{ overflowX: "auto" }}>
            <table className="cf-table">
              <thead>
                <tr>
                  <th>When</th>
                  <th>User</th>
                  <th>Event</th>
                  <th>Title</th>
                  <th>Decision</th>
                  <th>IP</th>
                </tr>
              </thead>
              <tbody>
                {activity.map((e) => (
                  <tr key={e.id}>
                    <td className="cf-faint" style={{ whiteSpace: "nowrap" }}>
                      {formatRelative(e.occurred_at)}
                    </td>
                    <td style={{ whiteSpace: "nowrap" }}>@{e.username}</td>
                    <td>
                      <EventPill type={e.event_type} />
                    </td>
                    <td>
                      {e.item_id ? (
                        <Link href={`/?title=${e.item_id}`}>
                          {e.title ?? `Item #${e.item_id}`}
                        </Link>
                      ) : (
                        <span className="cf-muted">
                          {e.title ??
                            (e.episode_id ? `Episode #${e.episode_id}` : "—")}
                        </span>
                      )}
                    </td>
                    <td>
                      {e.decision ? <DecisionPill decision={e.decision} /> : "—"}
                    </td>
                    <td className="cf-faint cf-mono">{e.ip ?? "unknown"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

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
    <div>
      <svg
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="none"
        style={{ display: "block", height: 128, width: "100%" }}
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
    <div>
      <svg
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="none"
        style={{ display: "block", height: 128, width: "100%" }}
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
      <div
        className="cf-flex cf-between cf-faint"
        style={{ fontSize: 10, marginTop: 6, padding: "0 2px" }}
      >
        {[0, 6, 12, 18].map((h) => (
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
    <div
      className="cf-flex cf-wrap cf-gap12 cf-faint"
      style={{ marginTop: 10, fontSize: 10, textTransform: "uppercase", letterSpacing: "0.06em" }}
    >
      {entries.map((e) => (
        <span key={e.label} className="cf-flex" style={{ gap: 6 }}>
          <span
            style={{
              display: "inline-block",
              height: 8,
              width: 8,
              borderRadius: 2,
              background: e.color,
            }}
            aria-hidden
          />
          {e.label}
        </span>
      ))}
    </div>
  );
}

/// One table row with a label + inline progress bar + percent, matching
/// the mockup's Top platforms layout. Used for the platforms list.
function BarRow({ label, pct }: { label: string; pct: number }) {
  return (
    <tr>
      <td>
        <div className="cf-flex cf-gap12">
          <span>{label}</span>
          <span
            className="cf-stat-bar"
            style={{ width: 90, margin: 0 }}
          >
            <i style={{ width: `${pct}%`, background: "var(--info)" }} />
          </span>
        </div>
      </td>
      <td className="cf-num">{pct}%</td>
    </tr>
  );
}

/// Compact minutes figure for the "Minutes watched" hero value: exact
/// up to 9,999, then "12k" / "184k" / "1.2M" so the tile stays tidy on
/// a busy server (mirrors the mockup's "184k"). Input is milliseconds.
function formatWatchMinutes(ms: number): string {
  const minutes = Math.round(ms / 60_000);
  if (minutes < 10_000) return minutes.toLocaleString();
  if (minutes < 1_000_000) return `${Math.round(minutes / 1000)}k`;
  return `${(minutes / 1_000_000).toFixed(1)}M`;
}

/// Whole-number hours from milliseconds, comma-grouped. Used for the
/// "≈ N hours" tile sub-line and the Top Users per-row figure.
function formatWatchHours(ms: number): number {
  return Math.round(ms / 3_600_000);
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
      aria-labelledby="user-drill-title"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 60,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        background: "rgba(0,0,0,0.7)",
        padding: 16,
      }}
    >
      <div
        className="cf-console"
        style={{
          width: "100%",
          maxWidth: 768,
          overflow: "hidden",
          borderRadius: 12,
          border: "1px solid var(--line-strong)",
          background: "var(--surface)",
          boxShadow: "0 24px 64px rgba(0,0,0,0.6)",
        }}
      >
        <div className="cf-drawer-head">
          <div style={{ flex: 1 }}>
            <div id="user-drill-title" className="cf-ttl">{label}</div>
            <div className="cf-sub">Most recent 100 events</div>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="cf-btn cf-ghost cf-sm"
          >
            ✕
          </button>
        </div>
        <div style={{ maxHeight: "70vh", overflowY: "auto" }}>
          {error && (
            <div className="cf-banner cf-err" style={{ margin: 16 }}>
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <circle cx="12" cy="12" r="9" />
                <path d="M12 8v4M12 16v.5" />
              </svg>
              <div>{error}</div>
            </div>
          )}
          {events == null ? (
            <LoadingPlaceholder />
          ) : events.length === 0 ? (
            <div
              className="cf-faint"
              style={{ padding: "32px 24px", textAlign: "center", fontSize: 13 }}
            >
              No events recorded for this user yet.
            </div>
          ) : (
            <table className="cf-table">
              <thead>
                <tr>
                  <th>When</th>
                  <th>Event</th>
                  <th>Title</th>
                  <th>Decision</th>
                  <th>IP</th>
                </tr>
              </thead>
              <tbody>
                {events.map((e) => (
                  <tr key={e.id}>
                    <td className="cf-faint" style={{ whiteSpace: "nowrap" }}>
                      {formatRelative(e.occurred_at)}
                    </td>
                    <td>
                      <EventPill type={e.event_type} />
                    </td>
                    <td>
                      {e.title ??
                        (e.episode_id
                          ? `Episode #${e.episode_id}`
                          : e.item_id
                            ? `Item #${e.item_id}`
                            : "—")}
                    </td>
                    <td>
                      {e.decision ? <DecisionPill decision={e.decision} /> : "—"}
                    </td>
                    <td className="cf-faint cf-mono">{e.ip ?? "unknown"}</td>
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
  tone,
  label,
  value,
  meta,
  icon,
  accessory,
}: {
  tone: string;
  label: string;
  value: string;
  meta: string;
  /// Inner SVG path(s); the wrapping <svg> + tone styling come from the
  /// console design system.
  icon: React.ReactNode;
  /// Optional element rendered to the right of value — used by the
  /// Now Playing tile to inline a concurrent-streams sparkline.
  accessory?: React.ReactNode;
}) {
  return (
    <div className={`cf-stat ${tone}`}>
      <div className="cf-stat-top">
        <span className="cf-stat-ico">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            {icon}
          </svg>
        </span>
        {label}
      </div>
      <div className="cf-flex cf-between" style={{ alignItems: "flex-end" }}>
        <div className="cf-stat-val">{value}</div>
        {accessory}
      </div>
      <div className="cf-stat-meta">{meta}</div>
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
      style={{ height: 20, width: 64, color: "var(--faint)" }}
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

function EmptyState({ children }: { children: React.ReactNode }) {
  return (
    <div
      className="cf-faint"
      style={{
        border: "1px dashed var(--line)",
        borderRadius: 8,
        background: "rgba(255,255,255,0.02)",
        padding: "24px 16px",
        textAlign: "center",
        fontSize: 12.5,
      }}
    >
      {children}
    </div>
  );
}

function EventPill({ type }: { type: StatsActivityRow["event_type"] }) {
  const toneByType: Record<string, string> = {
    start: "cf-ok",
    complete: "cf-info",
    pause: "cf-warn",
    resume: "cf-ok",
    stop: "",
    progress: "",
  };
  return (
    <span className={`cf-pill${toneByType[type] ? ` ${toneByType[type]}` : ""}`}>
      {type}
    </span>
  );
}

function DecisionPill({ decision }: { decision: "direct" | "transcode" }) {
  return decision === "direct" ? (
    <span className="cf-pill cf-ok">Direct</span>
  ) : (
    <span className="cf-pill cf-warn">Transcode</span>
  );
}

/// The now-playing stream tag — remux / direct play / HW transcode, colored
/// by treatment to match the mockup's per-row stream tag.
/// Mirrors streamTag() in AdminDashboardClient.tsx:
///   both-copy → Remux; software encoder → Direct play; otherwise HW transcode.
function TreatmentTag({ session: s }: { session: NowPlayingSession }) {
  const isRemux =
    s.video_treatment === "copy" && s.audio_treatment === "copy";
  const isSoftware = s.encoder.toLowerCase().includes("software");
  if (isRemux) {
    return <span className="cf-tag">Remux</span>;
  }
  if (isSoftware) {
    return <span className="cf-tag">Direct play</span>;
  }
  return (
    <span className="cf-tag" style={{ borderColor: "var(--info-soft)", color: "var(--info)" }}>
      HW transcode
    </span>
  );
}

/// Cycles the five console avatar tones so adjacent rows differ. Index
/// driven so a list keeps a stable, varied palette without per-user
/// color storage.
function avatarTone(i: number): string {
  return `cf-a${(i % 5) + 1}`;
}

/// First letter of a display name for the avatar bubble, upper-cased;
/// falls back to "?" for an empty name.
function initialOf(name: string): string {
  const c = name.trim().charAt(0);
  return c ? c.toUpperCase() : "?";
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
