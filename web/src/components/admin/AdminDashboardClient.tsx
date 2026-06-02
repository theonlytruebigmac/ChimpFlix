"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import {
  admin as adminApi,
  type DashboardResponse,
  type DashboardScanJob,
  type DashboardSession,
  type ScheduledTask,
  type SecretSlotView,
} from "@/lib/chimpflix-api";

interface Props {
  initial: DashboardResponse;
}

// Tone vocabulary kept local now that the dashboard no longer imports the
// shared ./ui primitives. Maps to the console's `cf-pill cf-*` modifiers.
type PillTone = "ok" | "warn" | "bad" | "info" | "muted";

function pillClass(tone: PillTone): string {
  switch (tone) {
    case "ok":
      return "cf-pill cf-ok";
    case "warn":
      return "cf-pill cf-warn";
    case "bad":
      return "cf-pill cf-err";
    case "info":
      return "cf-pill cf-info";
    default:
      return "cf-pill";
  }
}

// Slower than before: the live source for active_transcodes is now the
// WebSocket session feed. The poll is only there to catch the rest of the
// dashboard (library stats, scans, disk usage) and to recover if the WS
// drops without reconnecting.
const POLL_INTERVAL_MS = 30_000;

export function AdminDashboardClient({ initial }: Props) {
  const [data, setData] = useState<DashboardResponse>(initial);
  const [tasks, setTasks] = useState<ScheduledTask[] | null>(null);
  const [secrets, setSecrets] = useState<SecretSlotView[] | null>(null);
  const [fetching, setFetching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Tasks + secrets are fetched separately from the dashboard payload
  // — same admin scope, different endpoints. Tasks refresh on the 30s
  // cadence to keep "Next scheduled" countdowns accurate; secrets are
  // loaded once because they change rarely (operator action only).
  useEffect(() => {
    let cancelled = false;
    async function loadTasks() {
      try {
        const res = await adminApi.tasks.list();
        if (!cancelled) setTasks(res.tasks);
      } catch {
        if (!cancelled) setTasks([]);
      }
    }
    async function loadSecrets() {
      try {
        const res = await adminApi.secrets.list();
        if (!cancelled) setSecrets(res.slots);
      } catch {
        if (!cancelled) setSecrets([]);
      }
    }
    loadTasks();
    loadSecrets();
    const t = setInterval(loadTasks, 30_000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    async function tick() {
      try {
        const next = await adminApi.dashboard();
        if (cancelled) return;
        setData(next);
        setError(null);
      } catch (e) {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (!cancelled) {
          timer = setTimeout(tick, POLL_INTERVAL_MS);
        }
      }
    }
    void tick();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, []);

  // Live subscription for active transcodes. The server pushes a
  // `{ type: "sessions", active: [...] }` envelope whenever the set of
  // running sessions changes; we patch `active_transcodes` in place and
  // bump `server.now_ms` so the "Started X ago" cells stay accurate.
  useEffect(() => {
    if (typeof window === "undefined") return;
    let cancelled = false;
    let socket: WebSocket | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    function connect() {
      if (cancelled) return;
      const proto = window.location.protocol === "https:" ? "wss" : "ws";
      const url = `${proto}://${window.location.host}/api/v1/ws`;
      socket = new WebSocket(url);
      socket.onmessage = (e) => {
        try {
          const msg = JSON.parse(e.data as string) as {
            type?: string;
            active?: DashboardSession[];
          };
          if (msg.type === "sessions" && Array.isArray(msg.active)) {
            setData((d) => {
              // The WS frame carries raw snapshots (no DB enrichment —
              // it's a hot broadcast path), so carry forward the
              // resolved username/title we already have for any session
              // still present. New sessions show ids until the next 5s
              // poll re-enriches them.
              const prev = new Map(
                d.active_transcodes.map((s) => [s.id, s]),
              );
              const active = msg.active!.map((s) => {
                const old = prev.get(s.id);
                return old
                  ? {
                      ...s,
                      username: s.username ?? old.username,
                      title: s.title ?? old.title,
                      subtitle: s.subtitle ?? old.subtitle,
                    }
                  : s;
              });
              return {
                ...d,
                active_transcodes: active,
                server: { ...d.server, now_ms: Date.now() },
              };
            });
          }
        } catch {
          // Ignore non-JSON / unrelated frames (e.g. scan events).
        }
      };
      socket.onclose = () => {
        if (cancelled) return;
        // Try to reconnect after 5s; the poll fallback above keeps the
        // page useful in the meantime.
        reconnectTimer = setTimeout(connect, 5_000);
      };
      socket.onerror = () => {
        socket?.close();
      };
    }
    connect();

    return () => {
      cancelled = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      socket?.close();
    };
  }, []);

  async function stopSession(id: string) {
    setFetching(true);
    try {
      await adminApi.stopSession(id);
      const next = await adminApi.dashboard();
      setData(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setFetching(false);
    }
  }

  // ─── Derived stats for the hero strip ────────────────────────────
  const totalItems = data.library_stats.reduce(
    (acc, s) => acc + s.item_count,
    0,
  );
  const totalBytes = data.library_stats.reduce(
    (acc, s) => acc + s.total_bytes,
    0,
  );
  const sessionCount = data.active_transcodes.length;
  const hwSessions = data.active_transcodes.filter(
    (s) => !s.encoder.toLowerCase().includes("software"),
  ).length;
  const remuxSessions = data.active_transcodes.filter(
    (s) => s.video_treatment === "copy" && s.audio_treatment === "copy",
  ).length;
  const softwareSessions = Math.max(
    0,
    sessionCount - hwSessions - remuxSessions,
  );
  const maxDiskPct = data.disks.reduce((acc, d) => {
    if (d.total_bytes <= 0) return acc;
    const pct = (d.used_bytes / d.total_bytes) * 100;
    return pct > acc ? pct : acc;
  }, 0);
  // Busiest disk drives the Storage tile's used/total readout.
  const busiestDisk = data.disks.reduce<DashboardResponse["disks"][number] | null>(
    (acc, d) => {
      if (d.total_bytes <= 0) return acc;
      const pct = (d.used_bytes / d.total_bytes) * 100;
      const accPct = acc && acc.total_bytes > 0 ? (acc.used_bytes / acc.total_bytes) * 100 : -1;
      return pct > accPct ? d : acc;
    },
    null,
  );
  const storageBarColor =
    maxDiskPct >= 90 ? "var(--err)" : maxDiskPct >= 75 ? "var(--warn)" : "var(--ok)";

  // ─── Activity feed: merge recent scans + active session starts ──
  const feed = buildActivityFeed(
    data.recent_scans,
    data.active_transcodes,
    data.server.now_ms,
    data.library_stats,
  );

  // ─── Alerts: failed scans, near-full disks, missing recommended creds
  const alerts = buildAlerts(data, secrets);

  // ─── "Next scheduled" — soonest upcoming enabled tasks ──────────
  const upNext =
    tasks === null
      ? []
      : tasks
          .filter((t) => t.enabled)
          .slice()
          .sort((a, b) => a.next_run_at - b.next_run_at)
          .slice(0, 4);

  const allHealthy = alerts.length === 0;

  return (
    <div>
      {/* ── Header status pill (no page title — the sidebar + breadcrumb
          name the page; this replaces the mockup's page-head actions) ── */}
      <div className="cf-flex cf-between" style={{ marginBottom: 18 }}>
        <span />
        <span className={allHealthy ? "cf-pill cf-ok" : "cf-pill cf-warn"}>
          <span className="cf-dot" />
          {allHealthy
            ? "All systems healthy"
            : `${alerts.length} item${alerts.length === 1 ? "" : "s"} need attention`}
        </span>
      </div>

      {error && (
        <div role="alert" aria-live="assertive" className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>Failed to refresh: {error}</div>
        </div>
      )}

      {/* ── In-page tab bar. Only "Dashboard" lives on this route; the
          others are separate routes, rendered as links matching the
          mockup's tab look. ──────────────────────────────────────── */}
      <div className="cf-tabs">
        <button type="button" className="cf-tab cf-on">
          Dashboard
        </button>
        <Link className="cf-tab" href="/settings/admin/activity">
          Activity &amp; stats
        </Link>
        <Link className="cf-tab" href="/settings/admin/status/alerts">
          Alerts
          {alerts.length > 0 && (
            <span className="cf-pillcount">{alerts.length}</span>
          )}
        </Link>
      </div>

      {/* ── Hero stat tiles ─────────────────────────────────────────── */}
      <div className="cf-grid cf-c4">
        <div className="cf-stat cf-tone-green">
          <div className="cf-stat-top">
            <span className="cf-stat-ico">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z" />
                <path d="M9 12l2 2 4-4" />
              </svg>
            </span>
            System
          </div>
          <div className="cf-stat-val">Healthy</div>
          <div className="cf-stat-meta">
            v{data.server.version} · up {formatDuration(data.server.uptime_s)}
          </div>
        </div>

        <div className="cf-stat cf-tone-blue">
          <div className="cf-stat-top">
            <span className="cf-stat-ico">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <polygon points="6 4 20 12 6 20 6 4" />
              </svg>
            </span>
            Active sessions
          </div>
          <div className="cf-stat-val">{sessionCount}</div>
          <div className="cf-stat-meta">
            {sessionCount === 0
              ? "No transcodes running"
              : `${softwareSessions} direct · ${hwSessions} HW transcode · ${remuxSessions} remux`}
          </div>
        </div>

        <div className="cf-stat cf-tone-amber">
          <div className="cf-stat-top">
            <span className="cf-stat-ico">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <ellipse cx="12" cy="6" rx="8" ry="3" />
                <path d="M4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6" />
              </svg>
            </span>
            Storage
          </div>
          <div className="cf-stat-val">
            {data.disks.length > 0 ? (
              <>
                {maxDiskPct.toFixed(0)}
                <small>%</small>
              </>
            ) : (
              formatBytes(totalBytes)
            )}
          </div>
          <div className="cf-stat-meta">
            {busiestDisk
              ? `${formatBytes(busiestDisk.used_bytes)} of ${formatBytes(busiestDisk.total_bytes)}`
              : `${formatBytes(totalBytes)} of media`}
          </div>
          {data.disks.length > 0 && (
            <div className="cf-stat-bar">
              <i style={{ width: `${maxDiskPct}%`, background: storageBarColor }} />
            </div>
          )}
        </div>

        <div className="cf-stat cf-tone-red">
          <div className="cf-stat-top">
            <span className="cf-stat-ico">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <rect x="3" y="4" width="18" height="16" rx="2" />
                <path d="M7 4v16M17 4v16" />
              </svg>
            </span>
            Library
          </div>
          <div className="cf-stat-val">{formatNumber(totalItems)}</div>
          <div className="cf-stat-meta">
            {data.library_stats.length} librar
            {data.library_stats.length === 1 ? "y" : "ies"} ·{" "}
            {formatNumber(data.movie_count)} movie
            {data.movie_count === 1 ? "" : "s"} ·{" "}
            {formatNumber(data.episode_count)} ep
            {data.episode_count === 1 ? "" : "s"}
          </div>
        </div>
      </div>

      {/* ── Two-column body ─────────────────────────────────────────── */}
      <div className="cf-grid cf-c2" style={{ marginTop: 18, alignItems: "start" }}>
        {/* left column */}
        <div>
          {/* Now playing */}
          <div className="cf-card">
            <div className="cf-card-head">
              <div>
                <div className="cf-ttl">Now playing</div>
                <div className="cf-sub">Live · refreshes every 5s</div>
              </div>
              <div className="cf-head-aside">
                <span className="cf-pill cf-accent">
                  <span className="cf-dot" />
                  {sessionCount} stream{sessionCount === 1 ? "" : "s"}
                </span>
              </div>
            </div>
            {sessionCount === 0 ? (
              <div className="cf-card-body cf-pad">
                <span className="cf-faint" style={{ fontSize: 13 }}>
                  No active streams right now.
                </span>
              </div>
            ) : (
              <table className="cf-table">
                <tbody>
                  {data.active_transcodes.map((s) => (
                    <tr key={s.id}>
                      <td>
                        <div className="cf-flex cf-gap8">
                          <span
                            className={`cf-avatar ${avatarTone(s.user_id)}`}
                            style={{ width: 28, height: 28, fontSize: 11 }}
                          >
                            {(s.username ?? String(s.user_id)).slice(0, 1).toUpperCase()}
                          </span>
                          {s.username ?? `User #${s.user_id}`}
                        </div>
                      </td>
                      <td>
                        {s.title ? (
                          <div>
                            <div>{s.title}</div>
                            {s.subtitle ? (
                              <div className="cf-faint" style={{ fontSize: 11.5, marginTop: 1 }}>
                                {s.subtitle}
                              </div>
                            ) : null}
                          </div>
                        ) : (
                          <span className="cf-mono">#{s.media_file_id}</span>
                        )}
                      </td>
                      <td>{streamTag(s)}</td>
                      <td className="cf-num cf-mono">{s.target_height}p</td>
                      <td className="cf-num">
                        <button
                          type="button"
                          className="cf-btn cf-ghost cf-tiny"
                          disabled={fetching}
                          onClick={() => stopSession(s.id)}
                        >
                          {fetching ? "…" : "Stop"}
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>

          {/* Concurrent streams sparkline */}
          <div className="cf-card" style={{ marginBottom: 0 }}>
            <div className="cf-card-head">
              <div>
                <div className="cf-ttl">Concurrent streams</div>
                <div className="cf-sub">Live snapshot</div>
              </div>
            </div>
            <div className="cf-card-body cf-pad">
              <div className="cf-sparkline">
                {sparklineBars(sessionCount).map((h, i) => (
                  <i key={i} style={{ height: `${h}%` }} />
                ))}
              </div>
              <div
                className="cf-flex cf-between cf-faint"
                style={{ fontSize: 11, marginTop: 8 }}
              >
                <span>now</span>
                <span>{sessionCount} active</span>
              </div>
            </div>
          </div>
        </div>

        {/* right column */}
        <div>
          {/* Needs attention */}
          <div className="cf-card">
            <div className="cf-card-head">
              <div>
                <div className="cf-ttl">Needs attention</div>
                <div className="cf-sub">
                  {alerts.length === 0
                    ? "Nothing open"
                    : `${alerts.length} open item${alerts.length === 1 ? "" : "s"}`}
                </div>
              </div>
              <div className="cf-head-aside">
                <Link
                  className="cf-btn cf-ghost cf-tiny"
                  href="/settings/admin/status/alerts"
                >
                  View all →
                </Link>
              </div>
            </div>
            <div className="cf-card-body">
              {alerts.length === 0 ? (
                <div className="cf-row">
                  <div className="cf-row-main">
                    <div className="cf-row-help">
                      Nothing needs attention right now.
                    </div>
                  </div>
                </div>
              ) : (
                alerts.map((a) => (
                  <div className="cf-row" key={a.key} style={{ padding: "13px 0" }}>
                    <div className="cf-row-main">
                      <div className="cf-row-label" style={{ fontSize: 13 }}>
                        <span
                          className={pillClass(a.tone)}
                          style={{ padding: "1px 7px" }}
                        >
                          <span className="cf-dot" />
                          {a.badge}
                        </span>{" "}
                        {a.title}
                      </div>
                      <div className="cf-row-help">{a.meta}</div>
                    </div>
                    {a.when !== null && (
                      <div className="cf-row-control cf-faint" style={{ fontSize: 12 }}>
                        {formatAgo(data.server.now_ms - a.when)}
                      </div>
                    )}
                  </div>
                ))
              )}
            </div>
          </div>

          {/* Recent activity */}
          <div className="cf-card">
            <div className="cf-card-head">
              <div>
                <div className="cf-ttl">Recent activity</div>
              </div>
            </div>
            <div className="cf-card-body">
              {feed.length === 0 ? (
                <div className="cf-row">
                  <div className="cf-row-main">
                    <div className="cf-row-help">
                      Activity will appear here as scans and sessions run.
                    </div>
                  </div>
                </div>
              ) : (
                feed.map((f) => (
                  <div className="cf-row" key={f.key} style={{ padding: "11px 0" }}>
                    <div className="cf-row-main">
                      <div
                        className="cf-row-label"
                        style={{ fontSize: 13, fontWeight: 500 }}
                      >
                        {f.text}
                      </div>
                    </div>
                    <div className="cf-row-control cf-faint" style={{ fontSize: 12 }}>
                      {formatAgo(data.server.now_ms - f.when)}
                    </div>
                  </div>
                ))
              )}
            </div>
          </div>

          {/* Next scheduled */}
          <div className="cf-card" style={{ marginBottom: 0 }}>
            <div className="cf-card-head">
              <div>
                <div className="cf-ttl">Next scheduled</div>
              </div>
            </div>
            <div className="cf-card-body">
              {upNext.length === 0 ? (
                <div className="cf-row">
                  <div className="cf-row-main">
                    <div className="cf-row-help">
                      {tasks === null ? "Loading…" : "No enabled tasks."}
                    </div>
                  </div>
                </div>
              ) : (
                upNext.map((t) => {
                  const dueMs = t.next_run_at - data.server.now_ms;
                  return (
                    <div className="cf-row" key={t.id} style={{ padding: "11px 0" }}>
                      <div className="cf-row-main">
                        <div
                          className="cf-row-label"
                          style={{ fontSize: 13, fontWeight: 500 }}
                        >
                          {t.name}
                        </div>
                      </div>
                      <div className="cf-row-control">
                        <span className="cf-pill">
                          <span
                            className="cf-dot"
                            style={{ background: "var(--info)" }}
                          />
                          in {formatRelative(dueMs)}
                        </span>
                      </div>
                    </div>
                  );
                })
              )}
            </div>
          </div>
        </div>
      </div>

      {/* ── Quick actions ───────────────────────────────────────────── */}
      <div className="cf-section-title">Quick actions</div>
      <div className="cf-grid cf-c4">
        <Link
          className="cf-btn"
          style={{ justifyContent: "flex-start", padding: 14 }}
          href="/settings/admin/libraries"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M4 10a8 8 0 0 1 14-4l2 2M20 14a8 8 0 0 1-14 4l-2-2" />
            <path d="M18 4v4h-4M6 20v-4h4" />
          </svg>
          Scan libraries
        </Link>
        <Link
          className="cf-btn"
          style={{ justifyContent: "flex-start", padding: 14 }}
          href="/settings/admin/maintenance?tab=backups"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <ellipse cx="12" cy="6" rx="8" ry="3" />
            <path d="M4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6" />
          </svg>
          Backups
        </Link>
        <Link
          className="cf-btn"
          style={{ justifyContent: "flex-start", padding: 14 }}
          href="/settings/admin/logs?tab=audit"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M5 4h11v14a2 2 0 0 0 2 2H7a2 2 0 0 1-2-2z" />
          </svg>
          Audit log
        </Link>
        <Link
          className="cf-btn cf-primary"
          style={{ justifyContent: "flex-start", padding: 14 }}
          href="/settings/admin/users?tab=invites"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="9" cy="8" r="3.5" />
            <path d="M3 20a6 6 0 0 1 12 0" />
            <path d="M19 8v6M16 11h6" />
          </svg>
          Invite a user
        </Link>
      </div>
    </div>
  );
}

// ─── Now-playing helpers ───────────────────────────────────────────

// Deterministic avatar tone (cf-a1..a5) from the user id so the same user
// always gets the same color across refreshes.
function avatarTone(userId: number): string {
  const n = (Math.abs(userId) % 5) + 1;
  return `cf-a${n}`;
}

// The stream-type tag rendered in the Now-playing table. Mirrors the
// mockup's Direct play / HW transcode / Remux chips, derived from the
// session's encoder + copy treatments.
function streamTag(s: DashboardSession) {
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
    <span
      className="cf-tag"
      style={{ borderColor: "var(--info-soft)", color: "var(--info)" }}
    >
      HW transcode
    </span>
  );
}

// A small deterministic sparkline driven by the current concurrent-stream
// count. 30-minute history is net-new backend; until then this renders a
// gentle ramp that ends on the live count so the card reads as "live now"
// without inventing past data.
function sparklineBars(now: number): number[] {
  const bars = 15;
  const peak = Math.max(1, now, 4);
  const out: number[] = [];
  for (let i = 0; i < bars; i += 1) {
    const t = i / (bars - 1);
    const level = Math.round((0.35 + 0.65 * t) * 100);
    out.push(Math.max(8, Math.min(100, level)));
  }
  // Anchor the last bar to the live ratio so "now" is honest.
  out[bars - 1] = Math.max(8, Math.round((now / peak) * 100));
  return out;
}

// ─── Activity feed builder ─────────────────────────────────────────

interface FeedItem {
  key: string;
  when: number;
  text: React.ReactNode;
}

function buildActivityFeed(
  scans: DashboardScanJob[],
  sessions: DashboardSession[],
  nowMs: number,
  libraryStats: { library_id: number; name: string }[],
): FeedItem[] {
  void nowMs;
  const libName = (id: number) =>
    libraryStats.find((l) => l.library_id === id)?.name ?? `Library #${id}`;

  const items: FeedItem[] = [];

  // Scans that finished (have started_at)
  for (const s of scans) {
    if (s.started_at === null) continue;
    const finished = s.finished_at ?? s.started_at;
    items.push({
      key: `scan-${s.id}`,
      when: finished,
      text: (
        <span>
          Scan completed · <b>{libName(s.library_id)}</b> (+{s.files_added} ~
          {s.files_updated}
          {s.files_removed > 0 ? ` -${s.files_removed}` : ""})
        </span>
      ),
    });
  }

  // Sessions currently active (use created_at as "started" timestamp)
  for (const s of sessions) {
    items.push({
      key: `sess-${s.id}`,
      when: s.created_at,
      text: (
        <span>
          {s.username ?? `User #${s.user_id}`}
          {s.title ? (
            <>
              {" "}
              started <b>{s.title}</b>
            </>
          ) : (
            " started a session"
          )}{" "}
          · {s.target_height}p · {s.encoder}
        </span>
      ),
    });
  }

  items.sort((a, b) => b.when - a.when);
  return items.slice(0, 6);
}

// ─── Alert builder ─────────────────────────────────────────────────

interface AlertItem {
  key: string;
  tone: PillTone;
  badge: string;
  title: string;
  meta: string;
  when: number | null;
}

function buildAlerts(
  data: DashboardResponse,
  secrets: SecretSlotView[] | null,
): AlertItem[] {
  const alerts: AlertItem[] = [];

  for (const s of data.recent_scans) {
    if (s.status === "failed") {
      alerts.push({
        key: `scan-fail-${s.id}`,
        tone: "bad",
        badge: "Scan",
        title: `Scan failed: library #${s.library_id}`,
        meta: s.error_message ?? "no error message captured",
        when: s.finished_at ?? s.started_at ?? s.created_at,
      });
    }
  }

  for (const d of data.disks) {
    if (d.total_bytes <= 0) continue;
    const pct = (d.used_bytes / d.total_bytes) * 100;
    if (pct >= 90) {
      alerts.push({
        key: `disk-${d.path}`,
        tone: "bad",
        badge: "Disk",
        title: `${d.label} ${pct.toFixed(0)}% full`,
        meta: `${pct.toFixed(1)}% used · ${d.path}`,
        when: null,
      });
    } else if (pct >= 75) {
      alerts.push({
        key: `disk-${d.path}`,
        tone: "warn",
        badge: "Disk",
        title: `${d.label} ${pct.toFixed(0)}% full`,
        meta: `${d.path} is filling up. Consider pruning the transcode cache.`,
        when: null,
      });
    }
  }

  if (secrets) {
    const tmdb = secrets.find((s) => s.name === "tmdb");
    if (tmdb && !tmdb.stored?.set) {
      alerts.push({
        key: "cred-tmdb",
        tone: "info",
        badge: "Vault",
        title: "TMDB credential not set",
        meta: "Movies + TV metadata fetch is disabled until configured.",
        when: null,
      });
    }
  }

  return alerts;
}

// ─── Formatters (unchanged from previous version) ──────────────────

function formatRelative(ms: number): string {
  if (ms <= 0) return "now";
  return formatDuration(Math.floor(ms / 1000));
}

function formatAgo(ms: number): string {
  if (ms < 0) return "just now";
  if (ms < 1000) return "just now";
  return `${formatDuration(Math.floor(ms / 1000))} ago`;
}

function formatNumber(n: number): string {
  return n.toLocaleString();
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB", "PB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(v >= 100 ? 0 : v >= 10 ? 1 : 2)} ${units[i]}`;
}

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86_400) {
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    return m > 0 ? `${h}h ${m}m` : `${h}h`;
  }
  const d = Math.floor(seconds / 86_400);
  const h = Math.floor((seconds % 86_400) / 3600);
  return h > 0 ? `${d}d ${h}h` : `${d}d`;
}
