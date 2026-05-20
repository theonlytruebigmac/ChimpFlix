"use client";

import { useEffect, useRef, useState } from "react";
import {
  admin as adminApi,
  type JobRow,
  type JobStatusFilter,
  type JobSummary,
} from "@/lib/chimpflix-api";
import { Pill, FilterChip } from "./ui";

/// Owner-only queue dashboard. Three sections:
///
///   1. Hero strip — counts by status (queued / running / failed /
///      dead). Failed + dead are the actionable ones; queued and
///      running are informational.
///   2. Filter chip row — kind (all per-file kinds plus the legacy
///      item-level one), status filter.
///   3. Table — newest-first, last_error inline-wrapped for the dead
///      rows so you can see the failure reason without clicking in.
///
/// Refresh every 4s while the page is visible. Stops polling when
/// the tab is hidden (no point burning CPU + API hits when nobody
/// is looking).
const POLL_MS = 4_000;
const KINDS: { value: string; label: string }[] = [
  { value: "", label: "All kinds" },
  { value: "detect_markers_file", label: "Detect markers" },
  { value: "generate_preview_sprite", label: "Preview sprite" },
  { value: "build_chapter_thumbs", label: "Chapter thumbs" },
  { value: "analyze_loudness", label: "Loudness" },
];
const STATUSES: { value: "" | JobStatusFilter; label: string }[] = [
  { value: "", label: "All statuses" },
  { value: "queued", label: "Queued" },
  { value: "running", label: "Running" },
  { value: "failed", label: "Failed (retry pending)" },
  { value: "dead", label: "Dead (manual retry)" },
  { value: "succeeded", label: "Succeeded" },
];

export function AdminJobsClient({
  initialSummary,
  initialJobs,
}: {
  initialSummary: JobSummary;
  initialJobs: JobRow[];
}) {
  const [state, setState] = useState<{
    summary: JobSummary;
    jobs: JobRow[];
    nowMs: number;
  }>({ summary: initialSummary, jobs: initialJobs, nowMs: 0 });
  const [kind, setKind] = useState<string>("");
  const [status, setStatus] = useState<"" | JobStatusFilter>("");
  const [busyJobId, setBusyJobId] = useState<number | null>(null);
  const [sweepBusy, setSweepBusy] = useState(false);
  const [wipeBusy, setWipeBusy] = useState(false);
  const [sweepResult, setSweepResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Keep the alive flag in a ref so the polling interval sees the
  // latest value without re-creating the interval on every state
  // change (which would reset the cadence).
  const aliveRef = useRef(true);
  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  // Refetch on filter change + on the polling cadence. Both
  // pathways funnel through one effect so they can't race.
  useEffect(() => {
    let cancelled = false;
    async function refresh() {
      try {
        const [summary, list] = await Promise.all([
          adminApi.jobs.summary(),
          adminApi.jobs.list({
            kind: kind || undefined,
            status: status || undefined,
            limit: 100,
          }),
        ]);
        if (cancelled || !aliveRef.current) return;
        setState({ summary, jobs: list.jobs, nowMs: Date.now() });
        setError(null);
      } catch (e) {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      }
    }
    refresh();
    const interval = window.setInterval(() => {
      if (document.visibilityState !== "visible") return;
      void refresh();
    }, POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [kind, status]);

  async function processAllPending() {
    if (
      !window.confirm(
        "Sweep every file lacking a pipeline artifact (markers, preview sprite, chapter thumbs, loudness) and enqueue jobs for them? This is idempotent — re-running while jobs are in flight is safe — but on a large library it can enqueue tens of thousands of rows. Proceed?",
      )
    ) {
      return;
    }
    setSweepBusy(true);
    setError(null);
    setSweepResult(null);
    try {
      const counts = await adminApi.jobs.processAllPending();
      const total =
        counts.markers + counts.previews + counts.chapter_thumbs + counts.loudness;
      setSweepResult(
        total === 0
          ? "All files already have every artifact — nothing to enqueue."
          : `Enqueued ${total.toLocaleString()} jobs: ${counts.markers} markers, ${counts.previews} previews, ${counts.chapter_thumbs} chapter thumbs, ${counts.loudness} loudness.`,
      );
      // Refresh now so the summary counters jump immediately rather
      // than waiting for the next poll tick.
      const [summary, list] = await Promise.all([
        adminApi.jobs.summary(),
        adminApi.jobs.list({
          kind: kind || undefined,
          status: status || undefined,
          limit: 100,
        }),
      ]);
      setState({ summary, jobs: list.jobs, nowMs: Date.now() });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSweepBusy(false);
    }
  }

  async function wipeQueued() {
    if (state.summary.queued === 0) return;
    if (
      !window.confirm(
        `Delete all ${state.summary.queued.toLocaleString()} queued jobs? Running jobs will finish their current file but no more queued rows will be picked up. This is the "I clicked Process all pending by mistake" escape hatch.`,
      )
    ) {
      return;
    }
    setWipeBusy(true);
    setError(null);
    setSweepResult(null);
    try {
      const res = await adminApi.jobs.wipeQueued(kind || undefined);
      setSweepResult(
        `Wiped ${res.removed.toLocaleString()} queued job${res.removed === 1 ? "" : "s"}.`,
      );
      const [summary, list] = await Promise.all([
        adminApi.jobs.summary(),
        adminApi.jobs.list({
          kind: kind || undefined,
          status: status || undefined,
          limit: 100,
        }),
      ]);
      setState({ summary, jobs: list.jobs, nowMs: Date.now() });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setWipeBusy(false);
    }
  }

  async function requeue(jobId: number) {
    setBusyJobId(jobId);
    setError(null);
    try {
      await adminApi.jobs.requeue(jobId);
      const [summary, list] = await Promise.all([
        adminApi.jobs.summary(),
        adminApi.jobs.list({
          kind: kind || undefined,
          status: status || undefined,
          limit: 100,
        }),
      ]);
      // eslint-disable-next-line react-hooks/purity
      setState({ summary, jobs: list.jobs, nowMs: Date.now() });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyJobId(null);
    }
  }

  const { summary, jobs, nowMs } = state;

  return (
    <div className="space-y-5">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      {sweepResult && (
        <div className="rounded-md border border-sky-500/30 bg-sky-500/10 px-3 py-2 text-xs text-sky-200">
          {sweepResult}
        </div>
      )}

      {/* Hero counters */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-5">
        <Counter label="Queued" value={summary.queued} tone="info" />
        <Counter label="Running" value={summary.running} tone="accent" />
        <Counter label="Failed" value={summary.failed} tone="warn" />
        <Counter label="Dead" value={summary.dead} tone="bad" />
        <Counter label="Succeeded" value={summary.succeeded} tone="ok" />
      </div>

      {/* Backfill action — operator-triggered sweep that catches up
          files added before the discovery pipeline shipped, or any
          file whose on-discovery job died past max_attempts. */}
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-white/10 bg-white/2 px-4 py-3">
        <div>
          <div className="text-sm font-medium">Backfill existing library</div>
          <div className="text-xs text-white/55">
            Runs the discovery pipeline against every file already in the
            library that lacks an artifact. Use this once after upgrading;
            new files trigger the pipeline automatically on scan.
          </div>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => void wipeQueued()}
            disabled={wipeBusy || summary.queued === 0}
            className="rounded-md border border-red-500/30 bg-red-500/5 px-4 py-2 text-sm font-medium text-red-300 hover:border-red-500/55 hover:bg-red-500/10 disabled:opacity-50"
          >
            {wipeBusy ? "Wiping…" : `Wipe queued (${summary.queued})`}
          </button>
          <button
            type="button"
            onClick={() => void processAllPending()}
            disabled={sweepBusy}
            className="rounded-md border border-white/25 bg-white/5 px-4 py-2 text-sm font-medium text-white hover:border-white/45 hover:bg-white/10 disabled:opacity-50"
          >
            {sweepBusy ? "Sweeping…" : "Process all pending"}
          </button>
        </div>
      </div>

      {/* Filter chips */}
      <div className="space-y-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs uppercase tracking-wider text-white/45">
            Kind
          </span>
          {KINDS.map((k) => (
            <FilterChip
              key={k.value || "all"}
              active={kind === k.value}
              onClick={() => setKind(k.value)}
            >
              {k.label}
            </FilterChip>
          ))}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs uppercase tracking-wider text-white/45">
            Status
          </span>
          {STATUSES.map((s) => (
            <FilterChip
              key={s.value || "all"}
              active={status === s.value}
              onClick={() => setStatus(s.value)}
            >
              {s.label}
            </FilterChip>
          ))}
        </div>
      </div>

      {/* Table */}
      {jobs.length === 0 ? (
        <div className="rounded-lg border border-dashed border-white/15 bg-white/2 p-8 text-center text-sm text-white/55">
          No matching jobs.
        </div>
      ) : (
        <div className="overflow-hidden rounded-lg border border-white/10 bg-white/2">
          <table className="w-full text-sm">
            <thead className="bg-white/4 text-left text-[11.5px] uppercase tracking-wider text-white/45">
              <tr>
                <th className="w-16 px-3 py-2 font-semibold">#</th>
                <th className="px-3 py-2 font-semibold">Kind</th>
                <th className="px-3 py-2 font-semibold">Payload</th>
                <th className="w-24 px-3 py-2 font-semibold">Status</th>
                <th className="w-20 px-3 py-2 font-semibold">Attempts</th>
                <th className="w-32 px-3 py-2 font-semibold">Created</th>
                <th className="w-24 px-3 py-2"></th>
              </tr>
            </thead>
            <tbody>
              {jobs.map((j) => (
                <tr key={j.id} className="border-t border-white/6 align-top">
                  <td className="px-3 py-3 text-[12.5px] tabular-nums text-white/55">
                    {j.id}
                  </td>
                  <td className="px-3 py-3 text-[12.5px]">
                    <code className="font-mono text-[11.5px] text-white/80">
                      {j.kind}
                    </code>
                  </td>
                  <td className="px-3 py-3 text-[11.5px] text-white/55">
                    <code className="font-mono break-all">{j.payload}</code>
                    {j.last_error && (
                      <div className="mt-1 text-[11px] text-red-300/80">
                        {j.last_error}
                      </div>
                    )}
                  </td>
                  <td className="px-3 py-3">
                    <StatusPill status={j.status} />
                  </td>
                  <td className="px-3 py-3 text-[12.5px] tabular-nums text-white/65">
                    {j.attempts} / {j.max_attempts}
                  </td>
                  <td className="px-3 py-3 text-[12px] text-white/55">
                    {nowMs > 0
                      ? relativeSince(j.created_at, nowMs)
                      : new Date(j.created_at).toLocaleTimeString()}
                  </td>
                  <td className="px-3 py-3 text-right">
                    {(j.status === "dead" || j.status === "failed") && (
                      <button
                        type="button"
                        onClick={() => requeue(j.id)}
                        disabled={busyJobId === j.id}
                        className="rounded border border-white/20 px-2 py-1 text-[11px] text-white/80 hover:border-white/40 hover:text-white disabled:opacity-50"
                      >
                        {busyJobId === j.id ? "Requeuing…" : "Requeue"}
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <p className="text-xs text-white/45">
        Refreshes every {POLL_MS / 1000}s while the tab is visible. Failed
        rows are retry-pending — they re-claim automatically once their
        backoff window expires. Dead rows have exhausted{" "}
        <code className="font-mono">max_attempts</code> and need a manual
        Requeue.
      </p>
    </div>
  );
}

function Counter({
  label,
  value,
  tone,
}: {
  label: string;
  value: number;
  tone: "ok" | "warn" | "bad" | "info" | "accent" | "muted";
}) {
  const toneClass = {
    ok: "text-emerald-300",
    warn: "text-amber-300",
    bad: "text-red-300",
    info: "text-sky-300",
    accent: "text-(--color-accent)",
    muted: "text-white/70",
  }[tone];
  return (
    <div className="rounded-lg border border-white/10 bg-white/2 px-4 py-3">
      <div className="text-[11.5px] uppercase tracking-wider text-white/45">
        {label}
      </div>
      <div className={`mt-1 text-2xl font-semibold tabular-nums ${toneClass}`}>
        {value.toLocaleString()}
      </div>
    </div>
  );
}

function StatusPill({ status }: { status: JobStatusFilter }) {
  switch (status) {
    case "queued":
      return <Pill tone="info">queued</Pill>;
    case "running":
      return <Pill tone="accent">running</Pill>;
    case "succeeded":
      return <Pill tone="ok">succeeded</Pill>;
    case "failed":
      return <Pill tone="warn">failed</Pill>;
    case "dead":
      return <Pill tone="bad">dead</Pill>;
    default:
      return <Pill tone="muted">{status}</Pill>;
  }
}

function relativeSince(epochMs: number, nowMs: number): string {
  const diff = nowMs - epochMs;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} min ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}
