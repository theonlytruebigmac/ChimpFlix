"use client";

import { useEffect, useRef, useState } from "react";
import {
  admin as adminApi,
  type JobProgress,
  type JobRow,
  type JobStatusFilter,
  type JobSummary,
} from "@/lib/chimpflix-api";
import { DEFAULT_PAGE_SIZE, Pagination } from "./ui";
import { ConfirmDialog } from "../ConfirmDialog";
import { formatTime } from "@/lib/format";

/// Owner-only queue dashboard, styled in the console design language
/// (`cf-*`) to match the redesign mockup:
///
///   1. A status segmented control (All / Queued / Running / Failed /
///      Dead) + the maintenance action buttons.
///   2. A kind filter-pill row — "All kinds" plus the dynamic kind
///      list, auto-extending as new kinds are registered.
///   3. A `cf-table` — newest-first, last_error inline-wrapped for the
///      dead rows so the failure reason is visible without clicking in.
///
/// Refresh every 4s while the page is visible. Stops polling when the
/// tab is hidden (no point burning CPU + API hits when nobody looks).
const POLL_MS = 4_000;
/// Permanent "All kinds" chip plus the dynamic kind list — the
/// dynamic part is fetched from `/admin/tasks/activity` on mount so
/// it auto-extends as new job kinds are registered.
const ALL_KINDS_CHIP: { value: string; label: string } = {
  value: "",
  label: "All kinds",
};
const STATUSES: { value: "" | JobStatusFilter; label: string }[] = [
  { value: "", label: "All" },
  { value: "queued", label: "Queued" },
  { value: "running", label: "Running" },
  { value: "failed", label: "Failed" },
  { value: "dead", label: "Dead" },
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
  /// auto-extends to whatever kinds the binary actually has.
  const [kinds, setKinds] = useState<{ value: string; label: string }[]>([
    ALL_KINDS_CHIP,
  ]);
  const [busyJobId, setBusyJobId] = useState<number | null>(null);
  const [sweepBusy, setSweepBusy] = useState(false);
  const [wipeBusy, setWipeBusy] = useState(false);
  const [clearDeadBusy, setClearDeadBusy] = useState(false);
  const [sweepResult, setSweepResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Gating state for the three confirmation dialogs.
  const [askProcessAll, setAskProcessAll] = useState(false);
  const [askWipe, setAskWipe] = useState(false);
  const [askClearDead, setAskClearDead] = useState(false);
  const aliveRef = useRef(true);
  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  // One-shot kind discovery from the activity endpoint.
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
        // fallback — the table still works without per-kind chips.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Refetch on filter change + on the polling cadence. Both pathways
  // funnel through one effect so they can't race.
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

  async function reload() {
    const [summary, list] = await Promise.all([
      adminApi.jobs.summary(),
      adminApi.jobs.list({
        kind: kind || undefined,
        status: status || undefined,
        limit: pageSize,
        offset: (page - 1) * pageSize,
      }),
    ]);
    // Guard against setting state after the component unmounts (e.g. the
    // user navigates away while a mutation is still in flight).
    if (!aliveRef.current) return;
    setState({
      summary,
      jobs: list.jobs,
      total: list.total,
      progress: list.progress,
      // eslint-disable-next-line react-hooks/purity
      nowMs: Date.now(),
    });
  }

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
      await reload();
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
      await reload();
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
      await reload();
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
      await reload();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyJobId(null);
    }
  }

  const { summary, jobs, nowMs, progress } = state;

  return (
    <div>
      {error && (
        <div role="alert" aria-live="assertive" className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      {sweepResult && (
        <div className="cf-banner cf-info">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v.5M12 11v5" />
          </svg>
          <div>{sweepResult}</div>
        </div>
      )}

      {/* ── status segmented control + actions ────────────────────── */}
      <div
        className="cf-flex cf-between cf-wrap cf-gap8"
        style={{ marginBottom: 14 }}
      >
        <div className="cf-seg">
          {STATUSES.map((s) => (
            <button
              key={s.value || "all"}
              type="button"
              className={status === s.value ? "cf-on" : undefined}
              onClick={() => {
                setStatus(s.value);
                setPage(1);
              }}
            >
              {s.label}
            </button>
          ))}
        </div>
        <div className="cf-flex cf-gap8">
          <button
            type="button"
            className="cf-btn cf-sm"
            onClick={() => setAskProcessAll(true)}
            disabled={sweepBusy}
          >
            {sweepBusy ? "Sweeping…" : "Process all pending"}
          </button>
          <button
            type="button"
            className="cf-btn cf-danger cf-sm"
            onClick={() => setAskWipe(true)}
            disabled={wipeBusy || summary.queued === 0}
          >
            {wipeBusy ? "Wiping…" : `Wipe queued (${summary.queued})`}
          </button>
          <button
            type="button"
            className="cf-btn cf-danger cf-sm"
            onClick={() => setAskClearDead(true)}
            disabled={clearDeadBusy || summary.dead === 0}
          >
            {clearDeadBusy ? "Clearing…" : `Clear dead (${summary.dead})`}
          </button>
        </div>
      </div>

      {/* ── kind filter pills ─────────────────────────────────────── */}
      <div className="cf-flex cf-wrap cf-gap8" style={{ marginBottom: 14 }}>
        {kinds.map((k) => {
          const active = kind === k.value;
          return (
            <button
              key={k.value || "all"}
              type="button"
              className={`cf-pill${active ? " cf-accent" : ""}`}
              style={{ cursor: "pointer" }}
              onClick={() => {
                setKind(k.value);
                setPage(1);
              }}
            >
              {k.label}
            </button>
          );
        })}
      </div>

      {/* ── confirmation dialogs ──────────────────────────────────── */}
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
                safe — but on a large library it can enqueue tens of thousands
                of rows.
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
                Running jobs will finish their current file but no more queued
                rows will be picked up.
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
              <code className="font-mono text-white/85">max_attempts</code> (or
              have no handler at all — e.g. left over from a renamed kind). Use
              this when a Requeue won&rsquo;t help.
            </p>
          }
          confirmLabel="Clear dead"
          destructive
          busy={clearDeadBusy}
          onConfirm={() => void clearDead()}
          onCancel={() => setAskClearDead(false)}
        />
      )}

      {/* ── table ─────────────────────────────────────────────────── */}
      {jobs.length === 0 ? (
        <div className="cf-card" style={{ marginBottom: 0 }}>
          <div className="cf-card-body cf-pad cf-center cf-faint">
            No matching jobs.
          </div>
        </div>
      ) : (
        <div className="cf-card" style={{ marginBottom: 0 }}>
          <table className="cf-table">
            <thead>
              <tr>
                <th>Job</th>
                <th>Kind</th>
                <th>Status</th>
                <th>Attempts</th>
                <th>Payload / last error</th>
                <th>Created</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {jobs.map((j) => (
                <tr key={j.id}>
                  <td className="cf-mono">#{j.id}</td>
                  <td className="cf-mono">{j.kind}</td>
                  <td>
                    <StatusPill status={j.status} />
                  </td>
                  <td className="cf-num cf-mono">
                    {j.attempts} / {j.max_attempts}
                  </td>
                  <td>
                    <code
                      className="cf-mono"
                      style={{ wordBreak: "break-all" }}
                    >
                      {j.payload}
                    </code>
                    <LiveProgressLine progress={progress[String(j.id)]} />
                    <StageTimingsLine stageTimingsJson={j.stage_timings_json} />
                    {j.last_error && (
                      <div
                        className="cf-mono"
                        style={{
                          marginTop: 4,
                          fontSize: 11,
                          color: "#fca5a5",
                        }}
                      >
                        {j.last_error}
                      </div>
                    )}
                  </td>
                  <td className="cf-faint">
                    {nowMs > 0
                      ? relativeSince(j.created_at, nowMs)
                      : formatTime(j.created_at)}
                  </td>
                  <td className="cf-num">
                    {(j.status === "dead" || j.status === "failed") && (
                      <button
                        type="button"
                        className="cf-btn cf-ghost cf-tiny"
                        onClick={() => requeue(j.id)}
                        disabled={busyJobId === j.id}
                      >
                        {busyJobId === j.id ? "Requeuing…" : "Retry"}
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {state.total > pageSize && (
        <div style={{ marginTop: 16 }}>
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
        </div>
      )}

      <p className="cf-faint" style={{ fontSize: 12, marginTop: 14 }}>
        Refreshes every {POLL_MS / 1000}s while the tab is visible. Failed rows
        are retry-pending — they re-claim automatically once their backoff
        window expires. Dead rows have exhausted{" "}
        <code className="cf-mono">max_attempts</code> and need a manual Retry.
      </p>
    </div>
  );
}

function StatusPill({ status }: { status: JobStatusFilter }) {
  switch (status) {
    case "queued":
      return (
        <span className="cf-pill">
          <span className="cf-dot" style={{ background: "var(--ghost)" }} />
          Queued
        </span>
      );
    case "running":
      return (
        <span className="cf-pill cf-info">
          <span className="cf-dot" />
          Running
        </span>
      );
    case "succeeded":
      return (
        <span className="cf-pill cf-ok">
          <span className="cf-dot" />
          Succeeded
        </span>
      );
    case "failed":
      return (
        <span className="cf-pill cf-err">
          <span className="cf-dot" />
          Failed
        </span>
      );
    case "dead":
      return (
        <span
          className="cf-pill cf-err"
          style={{ background: "rgba(248,113,113,.2)" }}
        >
          <span className="cf-dot" />
          Dead
        </span>
      );
    default:
      return <span className="cf-pill">{status}</span>;
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
/// the job isn't currently executing.
function LiveProgressLine({ progress }: { progress: JobProgress | undefined }) {
  if (!progress) return null;
  const pct =
    progress.percent != null
      ? `${Math.round(progress.percent * 100)}%`
      : null;
  return (
    <div
      style={{
        marginTop: 4,
        display: "flex",
        alignItems: "center",
        gap: 6,
        fontSize: 11,
        color: "#7dd3fc",
      }}
    >
      <span
        className="animate-pulse"
        style={{
          display: "inline-block",
          height: 6,
          width: 6,
          borderRadius: "50%",
          background: "#38bdf8",
        }}
      />
      <span>
        {progress.stage}
        {pct && ` · ${pct}`}
      </span>
    </div>
  );
}

/// Inline stage-breakdown for jobs with a persisted timing blob.
/// Renders as "decode 3m 02s · fingerprint 1m 04s · loudness 67s".
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
    <div className="cf-faint" style={{ marginTop: 4, fontSize: 11 }}>
      {stages.map(([label, ms], i) => (
        <span key={label}>
          {i > 0 && <span style={{ margin: "0 6px" }}>·</span>}
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
