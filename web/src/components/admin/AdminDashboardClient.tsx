"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import {
  admin as adminApi,
  type DashboardResponse,
  type DashboardSession,
  type ScheduledTask,
  type SecretSlotView,
} from "@/lib/chimpflix-api";

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
    timer = setTimeout(tick, POLL_INTERVAL_MS);
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

  const totalItems = data.library_stats.reduce(
    (acc, s) => acc + s.item_count,
    0,
  );
  const totalFiles = data.library_stats.reduce(
    (acc, s) => acc + s.file_count,
    0,
  );
  const totalBytes = data.library_stats.reduce(
    (acc, s) => acc + s.total_bytes,
    0,
  );

  return (
    <div className="space-y-6">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          Failed to refresh: {error}
        </div>
      )}

      <section className="grid grid-cols-2 gap-3 sm:grid-cols-4 lg:grid-cols-6">
        <Stat label="Version" value={`v${data.server.version}`} />
        <Stat label="Uptime" value={formatDuration(data.server.uptime_s)} />
        <Stat label="Libraries" value={`${data.library_stats.length}`} />
        <Stat label="Items" value={formatNumber(totalItems)} />
        <Stat label="Files" value={formatNumber(totalFiles)} />
        <Stat label="Size" value={formatBytes(totalBytes)} />
      </section>

      <section>
        <SectionTitle
          title="Active transcodes"
          live={data.active_transcodes.length > 0}
        />
        {data.active_transcodes.length === 0 ? (
          <Empty>No active transcodes.</Empty>
        ) : (
          <div className="overflow-hidden rounded-lg border border-white/10">
            <table className="w-full text-sm">
              <thead className="bg-white/5 text-left text-xs uppercase tracking-wider text-white/40">
                <tr>
                  <th className="px-4 py-2">Session</th>
                  <th className="px-4 py-2">User</th>
                  <th className="px-4 py-2">File</th>
                  <th className="px-4 py-2">Resolution</th>
                  <th className="px-4 py-2">Encoder</th>
                  <th className="px-4 py-2">Started</th>
                  <th className="px-4 py-2">Last seen</th>
                  <th className="px-4 py-2" />
                </tr>
              </thead>
              <tbody>
                {data.active_transcodes.map((s) => (
                  <tr key={s.id} className="border-t border-white/5">
                    <td className="whitespace-nowrap px-4 py-2 font-mono text-xs">
                      {s.id}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/70">
                      #{s.user_id}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/70">
                      file #{s.media_file_id}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-xs text-white/70 tabular-nums">
                      <ResolutionCell
                        sourceHeight={s.source_height}
                        targetHeight={s.target_height}
                        bitrateBps={s.target_video_bitrate_bps}
                      />
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-xs text-white/70">
                      <EncoderChip
                        label={s.encoder}
                        videoTreatment={s.video_treatment}
                        audioTreatment={s.audio_treatment}
                      />
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/60">
                      {formatAgo(data.server.now_ms - s.created_at)}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/60">
                      {formatAgo(data.server.now_ms - s.last_seen_at)}
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
        )}
      </section>

      <section>
        <SectionTitle title="Scheduled tasks" />
        {tasks === null ? (
          <Empty>Loading…</Empty>
        ) : tasks.length === 0 ? (
          <Empty>
            No scheduled tasks. Add one under{" "}
            <Link
              href="/settings/admin/library/scheduled-tasks"
              className="underline hover:text-white"
            >
              Library → Scheduled Tasks
            </Link>
            .
          </Empty>
        ) : (
          <TaskSummary tasks={tasks} nowMs={data.server.now_ms} />
        )}
      </section>

      <section>
        <SectionTitle title="Credential vault" />
        {secrets === null ? (
          <Empty>Loading…</Empty>
        ) : secrets.length === 0 ? (
          <Empty>No credential slots registered.</Empty>
        ) : (
          <VaultSummary slots={secrets} />
        )}
      </section>

      <section>
        <SectionTitle title="Libraries" />
        {data.library_stats.length === 0 ? (
          <Empty>
            No libraries yet — add one under Library → Libraries.
          </Empty>
        ) : (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {data.library_stats.map((s) => (
              <div
                key={s.library_id}
                className="rounded-lg border border-white/10 bg-white/2 p-4"
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="font-medium">{s.name}</span>
                  <span className="rounded bg-white/10 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-white/60">
                    {s.kind}
                  </span>
                </div>
                <div className="mt-2 grid grid-cols-3 gap-2 text-xs text-white/60">
                  <div>
                    <div className="text-white">{formatNumber(s.item_count)}</div>
                    items
                  </div>
                  <div>
                    <div className="text-white">{formatNumber(s.file_count)}</div>
                    files
                  </div>
                  <div>
                    <div className="text-white">{formatBytes(s.total_bytes)}</div>
                    size
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}
      </section>

      <section>
        <SectionTitle title="Recent scans" />
        {data.recent_scans.length === 0 ? (
          <Empty>No scan jobs recorded.</Empty>
        ) : (
          <div className="overflow-hidden rounded-lg border border-white/10">
            <table className="w-full text-sm">
              <thead className="bg-white/5 text-left text-xs uppercase tracking-wider text-white/40">
                <tr>
                  <th className="px-4 py-2">Library</th>
                  <th className="px-4 py-2">Status</th>
                  <th className="px-4 py-2">Started</th>
                  <th className="px-4 py-2">Files</th>
                  <th className="px-4 py-2">Error</th>
                </tr>
              </thead>
              <tbody>
                {data.recent_scans.map((s) => (
                  <tr key={s.id} className="border-t border-white/5">
                    <td className="whitespace-nowrap px-4 py-2 text-white/70">
                      #{s.library_id}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2">
                      <StatusBadge status={s.status} />
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/60">
                      {s.started_at ? formatWhen(s.started_at) : "—"}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/70 tabular-nums">
                      +{s.files_added} / ~{s.files_updated} / −{s.files_removed}
                    </td>
                    <td className="px-4 py-2 text-xs text-red-300">
                      {s.error_message ?? ""}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <section>
        <SectionTitle title="Disk usage" />
        {data.disks.length === 0 ? (
          <Empty>No probable disks (paths missing or unreadable).</Empty>
        ) : (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            {data.disks.map((d) => {
              const pct =
                d.total_bytes > 0
                  ? Math.min(100, (d.used_bytes / d.total_bytes) * 100)
                  : 0;
              return (
                <div
                  key={d.path}
                  className="rounded-lg border border-white/10 bg-white/2 p-4"
                >
                  <div className="flex items-center justify-between gap-2 text-sm">
                    <span className="font-medium">{d.label}</span>
                    <span className="text-xs text-white/50">{d.path}</span>
                  </div>
                  <div className="mt-2 h-2 overflow-hidden rounded-full bg-white/10">
                    <div
                      className={`h-full ${pct > 90 ? "bg-red-500" : pct > 75 ? "bg-amber-400" : "bg-emerald-500"}`}
                      style={{ width: `${pct}%` }}
                    />
                  </div>
                  <div className="mt-1 text-xs text-white/60">
                    {formatBytes(d.used_bytes)} of {formatBytes(d.total_bytes)} (
                    {pct.toFixed(1)}%)
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </section>
    </div>
  );
}

function TaskSummary({
  tasks,
  nowMs,
}: {
  tasks: ScheduledTask[];
  nowMs: number;
}) {
  // Two columns side-by-side: "Up next" (soonest first, enabled only)
  // and "Recently run" (most recent last_run_at first, regardless of
  // status). Capped at 5 each so the card stays glanceable.
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
    <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
      <div className="rounded-lg border border-white/10 bg-white/2 p-4">
        <div className="mb-3 text-xs font-semibold uppercase tracking-wider text-white/45">
          Up next
        </div>
        {upNext.length === 0 ? (
          <div className="text-sm text-white/45">No enabled tasks.</div>
        ) : (
          <ul className="space-y-1.5 text-sm">
            {upNext.map((t) => (
              <li
                key={t.id}
                className="flex items-baseline justify-between gap-3"
              >
                <span className="truncate text-white/85">{t.name}</span>
                <span className="shrink-0 text-xs text-white/55">
                  in {formatRelative(t.next_run_at - nowMs)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="rounded-lg border border-white/10 bg-white/2 p-4">
        <div className="mb-3 text-xs font-semibold uppercase tracking-wider text-white/45">
          Recently run
        </div>
        {recent.length === 0 ? (
          <div className="text-sm text-white/45">No task runs yet.</div>
        ) : (
          <ul className="space-y-1.5 text-sm">
            {recent.map((t) => (
              <li
                key={t.id}
                className="flex items-baseline justify-between gap-3"
              >
                <span className="flex items-baseline gap-2 truncate text-white/85">
                  <TaskDot status={t.last_status} />
                  <span className="truncate">{t.name}</span>
                </span>
                <span className="shrink-0 text-xs text-white/55">
                  {t.last_run_at ? formatAgo(nowMs - t.last_run_at) : "—"}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function VaultSummary({ slots }: { slots: SecretSlotView[] }) {
  // Hide the system-managed session_hmac slot — it's not a user-facing
  // credential, just an internal key the server rotates. The remaining
  // slots are the integration agents the operator chose to wire up.
  const userSlots = slots.filter((s) => !s.managed);
  return (
    <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-4">
      {userSlots.map((slot) => {
        const set = Boolean(slot.stored?.set);
        return (
          <Link
            key={slot.name}
            href="/settings/admin/server/credentials"
            className="flex items-center justify-between gap-2 rounded-md border border-white/10 bg-white/2 px-3 py-2 text-sm transition-colors hover:bg-white/5"
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

/// Render a positive elapsed `ms` as just the duration (no suffix).
/// Used inside cells that already have "in X" / "X ago" framing
/// supplied by the surrounding copy. For an inline "X ago" string
/// suitable for tables, use `formatAgo` below.
function formatRelative(ms: number): string {
  if (ms <= 0) return "now";
  return formatDuration(Math.floor(ms / 1000));
}

function formatAgo(ms: number): string {
  if (ms < 0) return "just now";
  if (ms < 1000) return "just now";
  return `${formatDuration(Math.floor(ms / 1000))} ago`;
}

/// "Source → Target" cell with bitrate. Reads as "1080p → 720p · 2.5
/// Mbps" so the operator can see at a glance whether the session is
/// downscaling and how much bandwidth budget the encoder has. Bitrate
/// is shown in Mbps once the value is large enough to round cleanly;
/// kbps otherwise.
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

/// Pill cluster summarising what ffmpeg is doing for one session.
/// Three pieces: the encoder (emerald for hardware, muted for the
/// software fallback) plus an optional copy-status badge. When both
/// video and audio are copying it's a pure remux (no encoding at
/// all) — the cheapest possible path; we flag that explicitly so the
/// operator can see at a glance how light the session actually is.
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
    ? "bg-white/10 text-white/55"
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

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-white/10 bg-white/2 p-4">
      <div className="text-xs uppercase tracking-wider text-white/40">
        {label}
      </div>
      <div className="mt-1 text-xl font-semibold tabular-nums">{value}</div>
    </div>
  );
}

function SectionTitle({
  title,
  live = false,
}: {
  title: string;
  live?: boolean;
}) {
  return (
    <h2 className="mb-3 flex items-center gap-2 text-lg font-semibold">
      {title}
      {live && (
        <span className="flex items-center gap-1 rounded bg-emerald-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-emerald-300">
          <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-400" />
          Live
        </span>
      )}
    </h2>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-dashed border-white/15 bg-white/2 p-6 text-center text-sm text-white/50">
      {children}
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const cls =
    status === "completed"
      ? "bg-emerald-500/15 text-emerald-300"
      : status === "running"
        ? "bg-blue-500/15 text-blue-300"
        : status === "failed"
          ? "bg-red-500/15 text-red-300"
          : status === "queued"
            ? "bg-amber-500/15 text-amber-300"
            : "bg-white/10 text-white/60";
  return (
    <span
      className={`rounded px-1.5 py-0.5 text-[10px] uppercase tracking-wider ${cls}`}
    >
      {status}
    </span>
  );
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

function formatWhen(epochMs: number): string {
  return new Date(epochMs).toLocaleString();
}
