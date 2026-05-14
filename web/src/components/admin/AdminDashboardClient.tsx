"use client";

import { useEffect, useState } from "react";
import {
  admin as adminApi,
  type DashboardResponse,
  type DashboardSession,
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
  const [fetching, setFetching] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
                    <td className="whitespace-nowrap px-4 py-2 text-white/60">
                      {formatRelative(data.server.now_ms - s.created_at)}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/60">
                      {formatRelative(data.server.now_ms - s.last_seen_at)}
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

function formatRelative(ms: number): string {
  if (ms < 0) return "just now";
  if (ms < 1000) return "just now";
  return `${formatDuration(Math.floor(ms / 1000))} ago`;
}

function formatWhen(epochMs: number): string {
  return new Date(epochMs).toLocaleString();
}
