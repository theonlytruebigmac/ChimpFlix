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
import { HeroCard, Pill, StatusDot, type PillTone } from "./ui";

interface Props {
  initial: DashboardResponse;
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
  // cadence to keep "Up next" countdowns accurate; secrets are loaded
  // once because they change rarely (operator action only).
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
            setData((d) => ({
              ...d,
              active_transcodes: msg.active!,
              server: { ...d.server, now_ms: Date.now() },
            }));
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
  const maxDiskPct = data.disks.reduce((acc, d) => {
    if (d.total_bytes <= 0) return acc;
    const pct = (d.used_bytes / d.total_bytes) * 100;
    return pct > acc ? pct : acc;
  }, 0);
  const storageTone: PillTone =
    maxDiskPct >= 90 ? "bad" : maxDiskPct >= 75 ? "warn" : "ok";

  // ─── Activity feed: merge recent scans + active session starts ──
  const feed = buildActivityFeed(
    data.recent_scans,
    data.active_transcodes,
    data.server.now_ms,
    data.library_stats,
  );

  // ─── Alerts: failed scans, near-full disks, missing recommended creds
  const alerts = buildAlerts(data, secrets);

  return (
    <div className="space-y-6">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          Failed to refresh: {error}
        </div>
      )}

      {/* ── Hero strip ──────────────────────────────────────────── */}
      <section className="grid grid-cols-1 gap-3 md:grid-cols-3">
        <HeroCard
          tone="ok"
          label="System"
          icon={
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
              <path d="M20 6 9 17l-5-5" />
            </svg>
          }
          value="Healthy"
          meta={`v${data.server.version} · uptime ${formatDuration(data.server.uptime_s)} · ${data.library_stats.length} libraries`}
        />
        <HeroCard
          tone="info"
          label="Active sessions"
          icon={
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
              <polygon points="5 3 19 12 5 21 5 3" />
            </svg>
          }
          value={sessionCount === 0 ? "Idle" : sessionCount}
          meta={
            sessionCount === 0
              ? "No transcodes running"
              : `${hwSessions} hardware · ${sessionCount - hwSessions - remuxSessions} software · ${remuxSessions} remux`
          }
        />
        <HeroCard
          tone={storageTone}
          label="Storage"
          icon={
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <ellipse cx="12" cy="5" rx="9" ry="3" />
              <path d="M3 5v14a9 3 0 0 0 18 0V5" />
              <path d="M3 12a9 3 0 0 0 18 0" />
            </svg>
          }
          value={formatBytes(totalBytes)}
          meta={`${formatNumber(totalItems)} items · ${data.disks.length > 0 ? `${maxDiskPct.toFixed(0)}% on busiest disk` : "no disks probed"}`}
        />
      </section>

      {/* ── Activity + Alerts split ─────────────────────────────── */}
      <section className="grid grid-cols-1 gap-3 lg:grid-cols-[1.4fr_1fr]">
        <Card
          title="Recent activity"
          subtitle={
            feed.length === 0
              ? "Nothing yet"
              : `Latest ${feed.length} event${feed.length === 1 ? "" : "s"}`
          }
        >
          {feed.length === 0 ? (
            <EmptyInline>Activity will appear here as scans and sessions run.</EmptyInline>
          ) : (
            <ul className="divide-y divide-white/6">
              {feed.map((f) => (
                <li
                  key={f.key}
                  className="grid grid-cols-[28px_1fr_auto] items-center gap-2.5 px-4 py-2.5"
                >
                  <span className="grid h-7 w-7 place-items-center rounded-full bg-white/6 text-white/65">
                    {f.icon}
                  </span>
                  <span className="min-w-0 truncate text-[13px]">{f.text}</span>
                  <span className="shrink-0 text-[11.5px] text-white/45">
                    {formatAgo(data.server.now_ms - f.when)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card
          title="Alerts"
          subtitle="Surfaced from scans, scheduler, vault"
          aside={
            <Pill tone={alerts.length === 0 ? "muted" : "warn"} dot>
              {alerts.length === 0 ? "all clear" : `${alerts.length} open`}
            </Pill>
          }
        >
          {alerts.length === 0 ? (
            <EmptyInline>Nothing needs attention right now.</EmptyInline>
          ) : (
            <ul className="divide-y divide-white/6">
              {alerts.map((a) => (
                <li
                  key={a.key}
                  className="grid grid-cols-[10px_1fr_auto] items-start gap-2.5 px-4 py-3"
                >
                  <StatusDot tone={a.tone} className="mt-1.5" />
                  <div className="min-w-0">
                    <div className="text-[13px] font-medium">{a.title}</div>
                    <div className="text-[11.5px] text-white/50">{a.meta}</div>
                  </div>
                  {a.when !== null && (
                    <span className="shrink-0 text-[11.5px] text-white/40">
                      {formatAgo(data.server.now_ms - a.when)}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </Card>
      </section>

      {/* ── Quick actions ───────────────────────────────────────── */}
      <section className="grid grid-cols-2 gap-2.5 md:grid-cols-4">
        <QuickAction
          href="/settings/admin/library/libraries"
          title="Scan libraries"
          subtitle={`${data.library_stats.length} libraries`}
          icon={
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <polyline points="22 12 18 12 15 21 9 3 6 12 2 12" />
            </svg>
          }
        />
        <QuickAction
          href="/settings/admin/maintenance/backup"
          title="Backups"
          subtitle="VACUUM INTO + verify"
          icon={
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="12" cy="12" r="10" />
              <polyline points="12 6 12 12 16 14" />
            </svg>
          }
        />
        <QuickAction
          href="/settings/admin/maintenance/logs/audit"
          title="Audit log"
          subtitle="Recent admin actions"
          icon={
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="11" cy="11" r="8" />
              <path d="m21 21-4.3-4.3" />
            </svg>
          }
        />
        <QuickAction
          href="/settings/admin/users/invites"
          title="Invite a user"
          subtitle="Email or one-time link"
          icon={
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" />
              <circle cx="9" cy="7" r="4" />
              <path d="M19 8v6" />
              <path d="M22 11h-6" />
            </svg>
          }
        />
      </section>

      {/* ── Active transcodes table ─────────────────────────────── */}
      {data.active_transcodes.length > 0 && (
        <Card
          title="Active transcodes"
          aside={
            <Pill tone="ok" dot>
              live
            </Pill>
          }
        >
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead className="bg-white/4 text-left text-[11px] uppercase tracking-wider text-white/45">
                <tr>
                  <th className="px-4 py-2 font-semibold">Session</th>
                  <th className="px-4 py-2 font-semibold">User</th>
                  <th className="px-4 py-2 font-semibold">Resolution</th>
                  <th className="px-4 py-2 font-semibold">Encoder</th>
                  <th className="px-4 py-2 font-semibold">Started</th>
                  <th className="px-4 py-2" />
                </tr>
              </thead>
              <tbody>
                {data.active_transcodes.map((s) => (
                  <tr key={s.id} className="border-t border-white/6">
                    <td className="whitespace-nowrap px-4 py-2 font-mono text-xs text-white/80">
                      {s.id}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/70">
                      #{s.user_id}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-xs text-white/70 tabular-nums">
                      <ResolutionCell
                        sourceHeight={s.source_height}
                        targetHeight={s.target_height}
                        bitrateBps={s.target_video_bitrate_bps}
                      />
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-xs">
                      <EncoderChip
                        label={s.encoder}
                        videoTreatment={s.video_treatment}
                        audioTreatment={s.audio_treatment}
                      />
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/60">
                      {formatAgo(data.server.now_ms - s.created_at)}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-right">
                      <button
                        disabled={fetching}
                        onClick={() => stopSession(s.id)}
                        className="rounded border border-white/10 px-2 py-1 text-xs text-white/70 hover:bg-white/5 disabled:opacity-40"
                      >
                        Stop
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}

      {/* ── Tasks summary ───────────────────────────────────────── */}
      {tasks !== null && tasks.length > 0 && (
        <TaskSummary tasks={tasks} nowMs={data.server.now_ms} />
      )}

      {/* ── Libraries + disks ───────────────────────────────────── */}
      <section className="grid grid-cols-1 gap-3 lg:grid-cols-2">
        <Card title="Libraries" subtitle={`${data.library_stats.length} configured`}>
          {data.library_stats.length === 0 ? (
            <EmptyInline>
              No libraries yet — add one under{" "}
              <Link href="/settings/admin/library/libraries" className="underline hover:text-white">
                Library → Libraries
              </Link>
              .
            </EmptyInline>
          ) : (
            <ul className="divide-y divide-white/6">
              {data.library_stats.map((s) => (
                <li
                  key={s.library_id}
                  className="grid grid-cols-[1fr_auto_auto] items-center gap-3 px-4 py-2.5 text-[13px]"
                >
                  <span className="min-w-0 truncate">
                    <span className="font-medium">{s.name}</span>{" "}
                    <span className="text-white/45">· {s.kind}</span>
                  </span>
                  <span className="shrink-0 tabular-nums text-white/70">
                    {formatNumber(s.item_count)} items
                  </span>
                  <span className="shrink-0 tabular-nums text-white/55">
                    {formatBytes(s.total_bytes)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card title="Disk usage" subtitle={`${data.disks.length} mount${data.disks.length === 1 ? "" : "s"}`}>
          {data.disks.length === 0 ? (
            <EmptyInline>No probable disks (paths missing or unreadable).</EmptyInline>
          ) : (
            <ul className="divide-y divide-white/6">
              {data.disks.map((d) => {
                const pct =
                  d.total_bytes > 0
                    ? Math.min(100, (d.used_bytes / d.total_bytes) * 100)
                    : 0;
                const bar =
                  pct > 90 ? "bg-red-500" : pct > 75 ? "bg-amber-400" : "bg-emerald-500";
                return (
                  <li key={d.path} className="px-4 py-2.5">
                    <div className="flex items-center justify-between gap-2 text-[13px]">
                      <span className="min-w-0 truncate font-medium">{d.label}</span>
                      <span className="shrink-0 text-[11.5px] text-white/45">{pct.toFixed(1)}%</span>
                    </div>
                    <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-white/8">
                      <div className={`h-full ${bar}`} style={{ width: `${pct}%` }} />
                    </div>
                    <div className="mt-1 text-[11px] text-white/45">
                      {formatBytes(d.used_bytes)} of {formatBytes(d.total_bytes)}
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </Card>
      </section>

      {/* ── Credential vault summary ─────────────────────────────── */}
      {secrets !== null && secrets.filter((s) => !s.managed).length > 0 && (
        <Card title="Credential vault" subtitle="External integrations">
          <VaultSummary slots={secrets} />
        </Card>
      )}
    </div>
  );
}

// ─── Activity feed builder ─────────────────────────────────────────

interface FeedItem {
  key: string;
  when: number;
  icon: React.ReactNode;
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
      icon: (
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <polyline points="22 12 18 12 15 21 9 3 6 12 2 12" />
        </svg>
      ),
      text: (
        <span>
          <span className="text-white/90">Scan</span>{" "}
          <span className="text-white/55">on</span>{" "}
          <span className="text-white/90">{libName(s.library_id)}</span>
          <span className="text-white/55">
            {" "}— +{s.files_added} added · {s.files_updated} updated
            {s.files_removed > 0 ? ` · ${s.files_removed} removed` : ""}
          </span>
        </span>
      ),
    });
  }

  // Sessions currently active (use created_at as "started" timestamp)
  for (const s of sessions) {
    items.push({
      key: `sess-${s.id}`,
      when: s.created_at,
      icon: (
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <polygon points="5 3 19 12 5 21 5 3" />
        </svg>
      ),
      text: (
        <span>
          <span className="text-white/90">User #{s.user_id}</span>{" "}
          <span className="text-white/55">started a session ·</span>{" "}
          <span className="text-white/75">{s.target_height}p · {s.encoder}</span>
        </span>
      ),
    });
  }

  items.sort((a, b) => b.when - a.when);
  return items.slice(0, 8);
}

// ─── Alert builder ─────────────────────────────────────────────────

interface AlertItem {
  key: string;
  tone: PillTone;
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
        title: `Disk almost full: ${d.label}`,
        meta: `${pct.toFixed(1)}% used · ${d.path}`,
        when: null,
      });
    } else if (pct >= 75) {
      alerts.push({
        key: `disk-${d.path}`,
        tone: "warn",
        title: `Disk filling up: ${d.label}`,
        meta: `${pct.toFixed(1)}% used · ${d.path}`,
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
        title: "TMDB credential not set",
        meta: "Movies + TV metadata fetch is disabled until configured.",
        when: null,
      });
    }
  }

  return alerts;
}

// ─── Layout primitives kept local to the dashboard ─────────────────

function Card({
  title,
  subtitle,
  aside,
  children,
}: {
  title: string;
  subtitle?: string;
  aside?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="overflow-hidden rounded-lg border border-white/10 bg-white/2">
      <header className="flex items-baseline justify-between gap-3 border-b border-white/10 px-4 py-3">
        <div className="min-w-0">
          <div className="text-[13px] font-semibold">{title}</div>
          {subtitle && (
            <div className="text-[11.5px] text-white/50">{subtitle}</div>
          )}
        </div>
        {aside && <div className="shrink-0">{aside}</div>}
      </header>
      {children}
    </section>
  );
}

function EmptyInline({ children }: { children: React.ReactNode }) {
  return <div className="px-4 py-5 text-sm text-white/45">{children}</div>;
}

function QuickAction({
  href,
  title,
  subtitle,
  icon,
}: {
  href: string;
  title: string;
  subtitle: string;
  icon: React.ReactNode;
}) {
  return (
    <Link
      href={href}
      className="block rounded-lg border border-white/10 bg-white/2 p-4 text-left transition-all hover:-translate-y-px hover:border-white/20 hover:bg-white/4"
    >
      <span className="mb-2.5 grid h-8 w-8 place-items-center rounded-lg bg-accent/15 text-(--color-accent)">
        {icon}
      </span>
      <div className="text-[13px] font-semibold">{title}</div>
      <div className="text-[11.5px] text-white/50">{subtitle}</div>
    </Link>
  );
}

// ─── Task + vault summaries (kept from previous version) ───────────

function TaskSummary({
  tasks,
  nowMs,
}: {
  tasks: ScheduledTask[];
  nowMs: number;
}) {
  const upNext = tasks
    .filter((t) => t.enabled)
    .slice()
    .sort((a, b) => a.next_run_at - b.next_run_at)
    .slice(0, 5);
  const recent = tasks
    .filter((t) => t.last_run_at !== null)
    .slice()
    .sort((a, b) => (b.last_run_at ?? 0) - (a.last_run_at ?? 0))
    .slice(0, 5);
  return (
    <section className="grid grid-cols-1 gap-3 md:grid-cols-2">
      <Card title="Up next" subtitle="Soonest scheduled run">
        {upNext.length === 0 ? (
          <EmptyInline>No enabled tasks.</EmptyInline>
        ) : (
          <ul className="divide-y divide-white/6">
            {upNext.map((t) => (
              <li
                key={t.id}
                className="flex items-baseline justify-between gap-3 px-4 py-2 text-[13px]"
              >
                <span className="truncate text-white/85">{t.name}</span>
                <span className="shrink-0 text-[11.5px] text-white/55">
                  in {formatRelative(t.next_run_at - nowMs)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Card>
      <Card title="Recently run" subtitle="Most recent first">
        {recent.length === 0 ? (
          <EmptyInline>No task runs yet.</EmptyInline>
        ) : (
          <ul className="divide-y divide-white/6">
            {recent.map((t) => (
              <li
                key={t.id}
                className="flex items-baseline justify-between gap-3 px-4 py-2 text-[13px]"
              >
                <span className="flex min-w-0 items-baseline gap-2">
                  <TaskDot status={t.last_status} />
                  <span className="truncate text-white/85">{t.name}</span>
                </span>
                <span className="shrink-0 text-[11.5px] text-white/55">
                  {t.last_run_at ? formatAgo(nowMs - t.last_run_at) : "—"}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </section>
  );
}

function VaultSummary({ slots }: { slots: SecretSlotView[] }) {
  const userSlots = slots.filter((s) => !s.managed);
  return (
    <div className="grid grid-cols-2 gap-2 px-4 py-3 sm:grid-cols-3 lg:grid-cols-4">
      {userSlots.map((slot) => {
        const set = Boolean(slot.stored?.set);
        return (
          <Link
            key={slot.name}
            href="/settings/admin/server/credentials"
            className="flex items-center justify-between gap-2 rounded-md border border-white/10 bg-white/2 px-3 py-2 text-sm transition-colors hover:bg-white/4"
          >
            <span className="truncate text-white/85">{slot.display_name}</span>
            <span
              className={`flex shrink-0 items-center gap-1 text-xs ${
                set ? "text-emerald-300" : "text-white/40"
              }`}
              aria-label={set ? "Configured" : "Not configured"}
            >
              <span
                className={`inline-block h-1.5 w-1.5 rounded-full ${set ? "bg-emerald-400" : "bg-white/25"}`}
              />
              {set ? "Set" : "Empty"}
            </span>
          </Link>
        );
      })}
    </div>
  );
}

function TaskDot({ status }: { status: ScheduledTask["last_status"] }) {
  const cls =
    status === "success"
      ? "bg-emerald-400"
      : status === "failed"
        ? "bg-red-400"
        : status === "running"
          ? "bg-blue-400 animate-pulse"
          : "bg-white/30";
  return <span className={`inline-block h-2 w-2 shrink-0 rounded-full ${cls}`} />;
}

// ─── Encoder + resolution cells (kept from previous version) ───────

function ResolutionCell({
  sourceHeight,
  targetHeight,
  bitrateBps,
}: {
  sourceHeight: number | null;
  targetHeight: number;
  bitrateBps: number;
}) {
  const target = `${targetHeight}p`;
  const source = sourceHeight ? `${sourceHeight}p` : null;
  const rate = bitrateBps >= 1_000_000
    ? `${(bitrateBps / 1_000_000).toFixed(1).replace(/\.0$/, "")} Mbps`
    : `${Math.round(bitrateBps / 1000)} kbps`;
  return (
    <span className="inline-flex items-center gap-1.5">
      {source && source !== target ? (
        <>
          <span className="text-white/55">{source}</span>
          <span className="text-white/35">→</span>
          <span className="text-white/85">{target}</span>
        </>
      ) : (
        <span className="text-white/85">{target}</span>
      )}
      <span className="text-white/45">·</span>
      <span className="text-white/55">{rate}</span>
    </span>
  );
}

function EncoderChip({
  label,
  videoTreatment,
  audioTreatment,
}: {
  label: string;
  videoTreatment?: "copy" | "reencode";
  audioTreatment?: "copy" | "reencode";
}) {
  const isSoftware = label.toLowerCase().includes("software");
  const cls = isSoftware
    ? "bg-white/10 text-white/60"
    : "bg-emerald-500/15 text-emerald-300";
  const vCopy = videoTreatment === "copy";
  const aCopy = audioTreatment === "copy";
  const copyBadge = vCopy && aCopy
    ? { label: "Remux", title: "Both video and audio are being remuxed — no encoder running" }
    : vCopy
      ? { label: "V Copy", title: "Video stream is being remuxed, not re-encoded" }
      : aCopy
        ? { label: "A Copy", title: "Audio stream is being remuxed, not re-encoded" }
        : null;
  return (
    <span className="inline-flex items-center gap-1.5">
      <span
        className={`inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider ${cls}`}
      >
        {label}
      </span>
      {copyBadge && (
        <span
          className="inline-flex items-center rounded bg-sky-500/15 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider text-sky-300"
          title={copyBadge.title}
        >
          {copyBadge.label}
        </span>
      )}
    </span>
  );
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
