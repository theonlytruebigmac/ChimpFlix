"use client";

import { useEffect, useRef, useState } from "react";
import {
  admin as adminApi,
  type JobProgress,
  type JobRow,
  type JobStatusFilter,
  type JobSummary,
} from "@/lib/chimpflix-api";
import { DEFAULT_PAGE_SIZE, FilterChip, Pagination, Pill } from "./ui";
import { ConfirmDialog } from "../ConfirmDialog";

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
/// Permanent "All kinds" chip plus the dynamic kind list — the
/// dynamic part is fetched from `/admin/tasks/activity` on mount so
/// it auto-extends as new job kinds are registered. Used to be a
/// hardcoded 4-item list that went stale as kinds were added
/// (`bootstrap_season_refs`, `refresh_logos_item`, etc.), so jobs
/// from new kinds were invisible to the filter.
const ALL_KINDS_CHIP: { value: string; label: string } = {
  value: "",
  label: "All kinds",
};
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
    total: number;
    /// Per-job live progress keyed by job id. Empty on first
    /// render; populated by each poll tick.
    progress: Record<string, JobProgress>;
    nowMs: number;
  }>({
    summary: initialSummary,
    jobs: initialJobs,
    total: initialJobs.length,
    progress: {},
    nowMs: 0,
  });
  const [kind, setKind] = useState<string>("");
  const [status, setStatus] = useState<"" | JobStatusFilter>("");
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);
  /// Dynamic chip list — fetched from `/admin/tasks/activity` so it
  /// auto-extends to whatever kinds the binary actually has. Starts
  /// as just the "All kinds" chip; the effect below adds the rest.
  const [kinds, setKinds] = useState<{ value: string; label: string }[]>([
    ALL_KINDS_CHIP,
  ]);
  const [busyJobId, setBusyJobId] = useState<number | null>(null);
  const [sweepBusy, setSweepBusy] = useState(false);
  const [wipeBusy, setWipeBusy] = useState(false);
  const [clearDeadBusy, setClearDeadBusy] = useState(false);
  const [sweepResult, setSweepResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Gating state for the three confirmation dialogs. Booleans because
  // each action targets the whole current state (no per-row picking) —
  // no need for an object payload.
  const [askProcessAll, setAskProcessAll] = useState(false);
  const [askWipe, setAskWipe] = useState(false);
  const [askClearDead, setAskClearDead] = useState(false);
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

  // One-shot kind discovery. The activity endpoint already returns
  // every kind the server knows about (registry + any kind that's
  // ever shown up in the queue), each with a human display name —
  // exactly the shape we want for the filter chips. Sorted by
  // label so the chip order is stable across page loads.
  useEffect(() => {
    let cancelled = false;
    void adminApi.tasks
      .activity()
      .then((act) => {
        if (cancelled) return;
        const dynamic = act.per_kind
          .map((k) => ({ value: k.kind, label: k.display_name }))
          .sort((a, b) => a.label.localeCompare(b.label));
        setKinds([ALL_KINDS_CHIP, ...dynamic]);
      })
      .catch(() => {
        // Activity endpoint failed (rare). Stay on the "All kinds"
        // fallback — the filter is non-essential; the table still
        // works without per-kind chips.
      });
    return () => {
      cancelled = true;
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
            limit: pageSize,
          offset: (page - 1) * pageSize,
          }),
        ]);
        if (cancelled || !aliveRef.current) return;
        setState({
          summary,
          jobs: list.jobs,
          total: list.total,
          progress: list.progress,
          nowMs: Date.now(),
        });
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
  }, [kind, status, page, pageSize]);

  async function processAllPending() {
    setAskProcessAll(false);
    setSweepBusy(true);
    setError(null);
    setSweepResult(null);
    try {
      const counts = await adminApi.jobs.processAllPending();
      const total = counts.markers + counts.loudness;
      setSweepResult(
        total === 0
          ? "All files already have every artifact — nothing to enqueue."
          : `Enqueued ${total.toLocaleString()} jobs: ${counts.markers} markers, ${counts.loudness} loudness.`,
      );
      // Refresh now so the summary counters jump immediately rather
      // than waiting for the next poll tick.
      const [summary, list] = await Promise.all([
        adminApi.jobs.summary(),
        adminApi.jobs.list({
          kind: kind || undefined,
          status: status || undefined,
          limit: pageSize,
          offset: (page - 1) * pageSize,
        }),
      ]);
      setState({
        summary,
        jobs: list.jobs,
        total: list.total,
        progress: list.progress,
        nowMs: Date.now(),
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSweepBusy(false);
    }
  }

  async function wipeQueued() {
    if (state.summary.queued === 0) return;
    setAskWipe(false);
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
          limit: pageSize,
          offset: (page - 1) * pageSize,
        }),
      ]);
      setState({
        summary,
        jobs: list.jobs,
        total: list.total,
        progress: list.progress,
        nowMs: Date.now(),
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setWipeBusy(false);
    }
  }

  async function clearDead() {
    if (state.summary.dead === 0) return;
    setAskClearDead(false);
    setClearDeadBusy(true);
    setError(null);
    setSweepResult(null);
    try {
      const res = await adminApi.jobs.clearDead();
      setSweepResult(
        `Cleared ${res.removed.toLocaleString()} dead job${res.removed === 1 ? "" : "s"}.`,
      );
      const [summary, list] = await Promise.all([
        adminApi.jobs.summary(),
        adminApi.jobs.list({
          kind: kind || undefined,
          status: status || undefined,
          limit: pageSize,
          offset: (page - 1) * pageSize,
        }),
      ]);
      setState({
        summary,
        jobs: list.jobs,
        total: list.total,
        progress: list.progress,
        nowMs: Date.now(),
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setClearDeadBusy(false);
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
          limit: pageSize,
          offset: (page - 1) * pageSize,
        }),
      ]);
      setState({
        summary,
        jobs: list.jobs,
        total: list.total,
        progress: list.progress,
        // eslint-disable-next-line react-hooks/purity
        nowMs: Date.now(),
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyJobId(null);
    }
  }

  const { summary, jobs, nowMs, progress } = state;

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
            onClick={() => setAskClearDead(true)}
            disabled={clearDeadBusy || summary.dead === 0}
            className="rounded-md border border-red-500/30 bg-red-500/5 px-4 py-2 text-sm font-medium text-red-300 hover:border-red-500/55 hover:bg-red-500/10 disabled:opacity-50"
          >
            {clearDeadBusy ? "Clearing…" : `Clear dead (${summary.dead})`}
          </button>
          <button
            type="button"
            onClick={() => setAskWipe(true)}
            disabled={wipeBusy || summary.queued === 0}
            className="rounded-md border border-red-500/30 bg-red-500/5 px-4 py-2 text-sm font-medium text-red-300 hover:border-red-500/55 hover:bg-red-500/10 disabled:opacity-50"
          >
            {wipeBusy ? "Wiping…" : `Wipe queued (${summary.queued})`}
          </button>
          <button
            type="button"
            onClick={() => setAskProcessAll(true)}
            disabled={sweepBusy}
            className="rounded-md border border-white/25 bg-white/5 px-4 py-2 text-sm font-medium text-white hover:border-white/45 hover:bg-white/10 disabled:opacity-50"
          >
            {sweepBusy ? "Sweeping…" : "Process all pending"}
          </button>
        </div>
        {askProcessAll && (
          <ConfirmDialog
            title="Process all pending?"
            body={
              <>
                <p>
                  Sweep every file lacking a pipeline artifact (markers,
                  loudness) and enqueue jobs for them.
                </p>
                <p className="mt-2">
                  This is idempotent — re-running while jobs are in flight is
                  safe — but on a large library it can enqueue tens of
                  thousands of rows.
                </p>
              </>
            }
            confirmLabel="Process all"
            busy={sweepBusy}
            onConfirm={() => void processAllPending()}
            onCancel={() => setAskProcessAll(false)}
          />
        )}
        {askWipe && (
          <ConfirmDialog
            title={`Wipe ${summary.queued.toLocaleString()} queued job${summary.queued === 1 ? "" : "s"}?`}
            body={
              <>
                <p>
                  Running jobs will finish their current file but no more
                  queued rows will be picked up.
                </p>
                <p className="mt-2 text-white/55">
                  This is the &ldquo;I clicked Process all pending by
                  mistake&rdquo; escape hatch.
                </p>
              </>
            }
            confirmLabel="Wipe queued"
            destructive
            busy={wipeBusy}
            onConfirm={() => void wipeQueued()}
            onCancel={() => setAskWipe(false)}
          />
        )}
        {askClearDead && (
          <ConfirmDialog
            title={`Clear ${summary.dead.toLocaleString()} dead job${summary.dead === 1 ? "" : "s"}?`}
            body={
              <p>
                Dead rows have exhausted{" "}
                <code className="font-mono text-white/85">max_attempts</code>{" "}
                (or have no handler at all — e.g. left over from a renamed
                kind). Use this when a Requeue won&rsquo;t help.
              </p>
            }
            confirmLabel="Clear dead"
            destructive
            busy={clearDeadBusy}
            onConfirm={() => void clearDead()}
            onCancel={() => setAskClearDead(false)}
          />
        )}
      </div>

      {/* Filter chips */}
      <div className="space-y-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs uppercase tracking-wider text-white/45">
            Kind
          </span>
          {kinds.map((k) => (
            <FilterChip
              key={k.value || "all"}
              active={kind === k.value}
              onClick={() => {
                setKind(k.value);
                setPage(1);
              }}
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
              onClick={() => {
                setStatus(s.value);
                setPage(1);
              }}
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
                    {/*
                      Live progress for in-flight jobs and stage
                      breakdown for completed ones. Both are
                      operator-visibility nicety; never block
                      anything — render only when data is present.
                    */}
                    <LiveProgressLine progress={progress[String(j.id)]} />
                    <StageTimingsLine stageTimingsJson={j.stage_timings_json} />
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

      <Pagination
        page={page}
        pageSize={pageSize}
        total={state.total}
        onPageChange={setPage}
        onPageSizeChange={(s) => {
          setPageSize(s);
          setPage(1);
        }}
        noun="jobs"
      />

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

/// Inline live-progress pill for an in-flight job. Returns null when
/// the job isn't currently executing — the parent row renders without
/// any progress chrome.
function LiveProgressLine({ progress }: { progress: JobProgress | undefined }) {
  if (!progress) return null;
  const pct =
    progress.percent != null
      ? `${Math.round(progress.percent * 100)}%`
      : null;
  return (
    <div className="mt-1 flex items-center gap-2 text-[11px] text-sky-300/85">
      <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-sky-400" />
      <span>
        {progress.stage}
        {pct && ` · ${pct}`}
      </span>
    </div>
  );
}

/// Inline stage-breakdown for jobs with a persisted timing blob.
/// Renders as "decode 3m 02s · fingerprint 1m 04s · loudness 67s".
/// Falls back to null on parse failure (e.g. legacy blob shape).
function StageTimingsLine({
  stageTimingsJson,
}: {
  stageTimingsJson: string | null;
}) {
  if (!stageTimingsJson) return null;
  let parsed: Record<string, unknown>;
  try {
    parsed = JSON.parse(stageTimingsJson) as Record<string, unknown>;
  } catch {
    return null;
  }
  // Extract recognized fields; ignore unknown ones so the UI keeps
  // working as tacet's API grows.
  const stages: Array<[string, number]> = [];
  for (const [label, key] of [
    ["markers", "markers_ms"],
    ["fingerprint", "fingerprint_ms"],
    ["decode", "decode_ms"],
    ["loudness", "loudness_ms"],
  ] as const) {
    const v = parsed[key];
    if (typeof v === "number" && v > 0) {
      stages.push([label, v]);
    }
  }
  if (stages.length === 0) return null;
  return (
    <div className="mt-1 text-[11px] text-white/40">
      {stages.map(([label, ms], i) => (
        <span key={label}>
          {i > 0 && <span className="mx-1.5">·</span>}
          {label} {formatShortDuration(ms)}
        </span>
      ))}
    </div>
  );
}

function formatShortDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(s < 10 ? 1 : 0)}s`;
  const m = Math.floor(s / 60);
  const sec = Math.round(s % 60);
  return sec === 0 ? `${m}m` : `${m}m ${sec}s`;
}
