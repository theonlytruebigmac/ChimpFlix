"use client";

/// Activity screen (Screen 2 from `docs/pipelines/tasks-ui.html`).
/// Live per-kind health table, recent-runs feed, and the dead-letter
/// failure panel — all powered by `/admin/tasks/activity` with 5s
/// polling. The hero strip mirrors what the overview screen shows
/// but reads from the same `/summary` endpoint so a tab-switch is
/// instant.

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";

import {
  admin as adminApi,
  friendlyErrorMessage,
  type ActivityFailedJob,
  type ActivityKindHealth,
  type ActivityRecentRun,
  type TasksActivityResponse,
  type TasksSummaryResponse,
} from "@/lib/chimpflix-api";
import {
  formatDurationMs,
  formatRelativeAgo,
} from "@/lib/relative-time";
import {
  DEFAULT_PAGE_SIZE,
  HeroCard,
  Pagination,
  Pill,
  SaveBar,
  type PillTone,
} from "./ui";

interface Props {
  initialActivity: TasksActivityResponse;
  initialSummary: TasksSummaryResponse;
  /// `Date.now()` snapshot from the server fetch (see
  /// AdminTasksOverviewClient for the SSR-hydration motivation).
  initialNowMs: number;
  /// Raw JSON string from `server_settings.job_kind_concurrency` —
  /// already validated as a JSON object on the server. Parsed once
  /// into state for the per-kind cap editor.
  initialKindConcurrency: string;
}

const REFRESH_MS = 5_000;

/// Parse the `job_kind_concurrency` JSON. Tolerates a missing /
/// malformed payload by returning an empty map — the editor still
/// renders, just with every kind showing its registry default.
function parseOverrides(raw: string): Record<string, number> {
  try {
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      const out: Record<string, number> = {};
      for (const [k, v] of Object.entries(parsed)) {
        if (typeof v === "number" && Number.isFinite(v) && v >= 1) {
          out[k] = Math.floor(v);
        }
      }
      return out;
    }
  } catch {
    // fall through
  }
  return {};
}

export function AdminTasksActivityClient({
  initialActivity,
  initialSummary,
  initialNowMs,
  initialKindConcurrency,
}: Props) {
  const [activity, setActivity] = useState(initialActivity);
  const [summary, setSummary] = useState(initialSummary);
  const [nowMs, setNowMs] = useState(initialNowMs);
  const [error, setError] = useState<string | null>(null);
  // Per-kind concurrency editor state. Baseline mirrors what's
  // persisted; current is the live edits the operator hasn't saved
  // yet. Both keyed by `job_kind`; absent key = use registry default.
  const [capBaseline, setCapBaseline] = useState<Record<string, number>>(() =>
    parseOverrides(initialKindConcurrency),
  );
  const [capOverrides, setCapOverrides] = useState<Record<string, number>>(() =>
    parseOverrides(initialKindConcurrency),
  );

  const refresh = useCallback(async () => {
    try {
      const [a, s] = await Promise.all([
        adminApi.tasks.activity(),
        adminApi.tasks.summary(),
      ]);
      setActivity(a);
      setSummary(s);
      setNowMs(Date.now());
      setError(null);
    } catch (e) {
      setError(friendlyErrorMessage(e));
    }
  }, []);

  useEffect(() => {
    const id = setInterval(refresh, REFRESH_MS);
    return () => clearInterval(id);
  }, [refresh]);

  return (
    <div className="space-y-6">
      {error && (
        <div className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-300">
          {error}
        </div>
      )}

      <HeroStrip summary={summary} activity={activity} />

      <div className="flex items-center justify-end gap-2 text-xs text-white/60">
        <Link
          href="/settings/admin/library/scheduled-tasks"
          className="rounded border border-white/15 px-2.5 py-1 transition-colors hover:bg-white/5"
        >
          ← Back to tasks
        </Link>
        <Link
          href="/settings/admin/library/scheduled-tasks/queue"
          className="rounded border border-white/15 px-2.5 py-1 transition-colors hover:bg-white/5"
        >
          Job queue
        </Link>
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[1.5fr_1fr]">
        <PerKindHealthCard rows={activity.per_kind} />
        <RecentRunsCard runs={activity.recent_runs} nowMs={nowMs} />
      </div>

      <ConcurrencyEditorCard
        rows={activity.per_kind}
        overrides={capOverrides}
        onChange={setCapOverrides}
      />

      <FailedJobsCard failed={activity.failed} nowMs={nowMs} />

      <SaveBar
        dirtyCount={countDirtyOverrides(capBaseline, capOverrides)}
        summary="per-kind concurrency caps"
        onDiscard={() => setCapOverrides(capBaseline)}
        onSave={async () => {
          // Send only kinds that diverge from the registry default,
          // so the stored JSON stays small and a future registry
          // bump to a higher default applies automatically. Build
          // off `activity.per_kind` for the default lookup.
          const defaults: Record<string, number> = {};
          for (const r of activity.per_kind) {
            defaults[r.kind] = r.default_concurrency;
          }
          const payload: Record<string, number> = {};
          for (const [k, v] of Object.entries(capOverrides)) {
            if (defaults[k] !== v) payload[k] = v;
          }
          await adminApi.settings.patch({
            job_kind_concurrency: JSON.stringify(payload),
          });
          setCapBaseline(capOverrides);
        }}
      />
    </div>
  );
}

/// Resolve the effective cap for a kind: explicit override wins,
/// otherwise the registry default. The editor displays this number
/// (vs the registry default in the hint) so the operator always
/// sees the *active* value.
function effectiveCap(
  row: ActivityKindHealth,
  overrides: Record<string, number>,
): number {
  return overrides[row.kind] ?? row.default_concurrency;
}

/// SaveBar dirty count = number of kinds whose effective value
/// differs from the baseline. We only count kinds, not transitions
/// (e.g. setting a value back to the default removes the key from
/// the payload but still counts as a saved edit).
function countDirtyOverrides(
  baseline: Record<string, number>,
  current: Record<string, number>,
): number {
  const keys = new Set([...Object.keys(baseline), ...Object.keys(current)]);
  let n = 0;
  for (const k of keys) {
    if (baseline[k] !== current[k]) n++;
  }
  return n;
}

// ─── Hero strip ────────────────────────────────────────────────────────

function HeroStrip({
  summary,
  activity,
}: {
  summary: TasksSummaryResponse;
  activity: TasksActivityResponse;
}) {
  // Throughput is summed from the per-kind rows so the hero number
  // matches the table below; safer than maintaining a parallel
  // counter.
  const throughput = activity.per_kind.reduce(
    (acc, k) => acc + k.jobs_per_minute,
    0,
  );
  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
      <HeroCard
        tone="info"
        label="In-flight"
        value={summary.running.toString()}
        meta="across all kinds"
      />
      <HeroCard
        tone="muted"
        label="Queue depth"
        value={summary.queued.toLocaleString()}
        meta="pending"
      />
      <HeroCard
        tone="ok"
        label="Throughput"
        value={throughput.toFixed(1)}
        meta="jobs/min (process-life avg)"
      />
      <HeroCard
        tone={summary.failed_24h > 0 ? "bad" : "muted"}
        label="Failed last 24h"
        value={summary.failed_24h.toString()}
        meta={summary.failed_24h === 0 ? "—" : "see failure log below"}
      />
    </div>
  );
}

// ─── Per-kind health ───────────────────────────────────────────────────

function PerKindHealthCard({ rows }: { rows: ActivityKindHealth[] }) {
  return (
    <div className="overflow-hidden rounded-lg border border-white/10 bg-white/2">
      <div className="flex items-baseline justify-between border-b border-white/8 px-4 py-3">
        <div>
          <div className="text-[13.5px] font-semibold text-white/95">
            Per-kind health
          </div>
          <div className="text-xs text-white/55">
            Live counters · p95 from in-memory ring (last 100 runs)
          </div>
        </div>
      </div>
      <div className="grid grid-cols-[1fr_70px_70px_70px_80px_90px_70px] border-b border-white/8 bg-white/3 px-3 py-2 text-[10.5px] font-semibold uppercase tracking-[0.07em] text-white/55">
        <span>Kind</span>
        <span className="text-right">Queue</span>
        <span className="text-right">In-flight</span>
        <span className="text-right">Per min</span>
        <span className="text-right">p95</span>
        <span className="text-right">ETA</span>
        <span className="text-right">Errors</span>
      </div>
      {rows.length === 0 ? (
        <div className="px-4 py-6 text-center text-sm text-white/45">
          No kinds active.
        </div>
      ) : (
        rows.map((r) => <PerKindHealthRow key={r.kind} row={r} />)
      )}
    </div>
  );
}

function PerKindHealthRow({ row }: { row: ActivityKindHealth }) {
  return (
    <div className="grid grid-cols-[1fr_70px_70px_70px_80px_90px_70px] items-center gap-2 border-b border-white/8 px-3 py-2 text-[12.5px] last:border-b-0">
      <span className="min-w-0 truncate font-medium text-white/95">
        {row.display_name}
        <span className="ml-2 font-mono text-[10.5px] text-white/40">
          {row.kind}
        </span>
      </span>
      <span className="text-right font-mono tabular-nums text-white/65">
        {row.queue_depth}
      </span>
      <span className="text-right font-mono tabular-nums text-white/65">
        {row.in_flight}
      </span>
      <span className="text-right font-mono tabular-nums text-white/65">
        {row.jobs_per_minute.toFixed(1)}
      </span>
      <span className="text-right font-mono tabular-nums text-white/65">
        {row.p95_duration_ms == null
          ? "—"
          : formatDurationMs(row.p95_duration_ms)}
      </span>
      <span
        className="text-right font-mono tabular-nums text-white/65"
        title={
          row.eta_seconds_remaining == null
            ? row.queue_depth === 0
              ? "Queue empty — nothing to drain."
              : "Not enough run history yet to estimate (need 5+ successful runs)."
            : `${row.queue_depth} queued × p95 ÷ effective concurrency. Coarse estimate; assumes current throughput.`
        }
      >
        {formatEta(row.eta_seconds_remaining)}
      </span>
      <span
        className={`text-right font-mono tabular-nums ${
          row.recent_errors > 0 ? "text-red-300" : "text-white/40"
        }`}
      >
        {row.recent_errors}
      </span>
    </div>
  );
}

/// Render a wall-clock ETA as a short human label: "—" / "~12s" /
/// "~4m" / "~2h 15m" / "~3d 4h". Deliberately coarse — the underlying
/// estimate is queue × p95 with all the noise that implies, so
/// minute-precision past an hour would be false confidence.
function formatEta(seconds: number | null): string {
  if (seconds == null) return "—";
  if (seconds < 60) return `~${Math.max(1, Math.round(seconds))}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `~${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const mins = minutes % 60;
  if (hours < 24) return mins === 0 ? `~${hours}h` : `~${hours}h ${mins}m`;
  const days = Math.floor(hours / 24);
  const hr = hours % 24;
  return hr === 0 ? `~${days}d` : `~${days}d ${hr}h`;
}

// ─── Per-kind concurrency editor ───────────────────────────────────────

function ConcurrencyEditorCard({
  rows,
  overrides,
  onChange,
}: {
  rows: ActivityKindHealth[];
  overrides: Record<string, number>;
  onChange: (next: Record<string, number>) => void;
}) {
  // Filter to registry-known kinds only — legacy custom-cron rows
  // surface with default_concurrency=1 and aren't actually capped
  // by `KindLimiter`. Surfacing them here would imply editability
  // we can't honour at the worker layer.
  const eligible = rows.filter((r) => r.default_concurrency >= 1);
  if (eligible.length === 0) return null;

  function setCapFor(kind: string, value: number, fallback: number) {
    const next = { ...overrides };
    // Setting to the registry default removes the key — keeps the
    // stored JSON minimal. The SaveBar still flags this as a dirty
    // change so the operator sees "reset" feedback.
    if (value === fallback) {
      delete next[kind];
    } else {
      next[kind] = value;
    }
    onChange(next);
  }

  return (
    <div className="overflow-hidden rounded-lg border border-white/10 bg-white/2">
      <div className="border-b border-white/8 px-4 py-3">
        <div className="text-[13.5px] font-semibold text-white/95">
          Per-kind concurrency
        </div>
        <div className="text-xs text-white/55">
          Raise the cap on a kind to let more of it run in parallel.
          Defaults are conservative — bump CPU-heavy kinds (markers /
          preview) on hefty boxes. Saved changes apply live; no
          restart required.
        </div>
      </div>
      <div className="grid grid-cols-[1fr_120px_90px] border-b border-white/8 bg-white/3 px-3 py-2 text-[10.5px] font-semibold uppercase tracking-[0.07em] text-white/55">
        <span>Kind</span>
        <span className="text-right">Default</span>
        <span className="text-right">Cap</span>
      </div>
      {eligible.map((r) => {
        const value = effectiveCap(r, overrides);
        const isOverride = overrides[r.kind] !== undefined;
        return (
          <div
            key={r.kind}
            className="grid grid-cols-[1fr_120px_90px] items-center gap-2 border-b border-white/8 px-3 py-2 text-[12.5px] last:border-b-0"
          >
            <span className="min-w-0 truncate font-medium text-white/95">
              {r.display_name}
              <span className="ml-2 font-mono text-[10.5px] text-white/40">
                {r.kind}
              </span>
            </span>
            <span className="text-right font-mono tabular-nums text-white/55">
              {r.default_concurrency}
              {isOverride && (
                <span className="ml-2 text-[10px] uppercase tracking-wider text-amber-300/80">
                  overridden
                </span>
              )}
            </span>
            <span className="flex justify-end">
              <input
                type="number"
                min={1}
                max={32}
                value={value}
                onChange={(e) => {
                  const n = Math.max(
                    1,
                    Math.min(32, Number.parseInt(e.target.value, 10) || 1),
                  );
                  setCapFor(r.kind, n, r.default_concurrency);
                }}
                className="w-16 rounded border border-white/15 bg-black/20 px-2 py-1 text-right font-mono text-[12.5px] text-white/90 focus:border-accent focus:outline-none"
                aria-label={`Concurrency cap for ${r.display_name}`}
              />
            </span>
          </div>
        );
      })}
    </div>
  );
}

// ─── Recent runs feed ──────────────────────────────────────────────────

function RecentRunsCard({
  runs,
  nowMs,
}: {
  runs: ActivityRecentRun[];
  nowMs: number;
}) {
  // Client-side pagination — the server already caps at 200 entries
  // (the live ring buffer's natural upper bound) so we slice the
  // visible chunk locally instead of round-tripping per page change.
  // Newest entries are always at the top; pagination here is about
  // scanning backward through the recent past, not jumping into
  // historical archive (that lives in `task_kind_metrics_daily`).
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);
  // Reset to page 1 if a refresh dropped the runs count below the
  // current page's offset (rare — happens on process restart).
  const totalPages = Math.max(1, Math.ceil(runs.length / pageSize));
  const effectivePage = Math.min(page, totalPages);
  const start = (effectivePage - 1) * pageSize;
  const slice = runs.slice(start, start + pageSize);

  return (
    <div className="overflow-hidden rounded-lg border border-white/10 bg-white/2">
      <div className="flex items-baseline justify-between border-b border-white/8 px-4 py-3">
        <div>
          <div className="text-[13.5px] font-semibold text-white/95">
            Live feed
          </div>
          <div className="text-xs text-white/55">
            Job completions, newest first
          </div>
        </div>
        <Pill tone="ok" dot>
          Live
        </Pill>
      </div>
      {runs.length === 0 ? (
        <div className="px-4 py-6 text-center text-sm text-white/45">
          No completed runs since process started.
        </div>
      ) : (
        <>
          {slice.map((r) => (
            <RecentRunRow
              key={`${r.kind}-${r.finished_at_ms}`}
              run={r}
              nowMs={nowMs}
            />
          ))}
          {runs.length > pageSize && (
            <div className="border-t border-white/8 px-4 py-2">
              <Pagination
                page={effectivePage}
                pageSize={pageSize}
                total={runs.length}
                onPageChange={setPage}
                onPageSizeChange={(s) => {
                  setPageSize(s);
                  setPage(1);
                }}
                noun="runs"
              />
            </div>
          )}
        </>
      )}
    </div>
  );
}

function RecentRunRow({
  run,
  nowMs,
}: {
  run: ActivityRecentRun;
  nowMs: number;
}) {
  const tone: PillTone = run.success
    ? "ok"
    : run.error_class === "external_rate_limit"
      ? "warn"
      : "bad";
  return (
    <div className="grid grid-cols-[8px_1fr_auto] items-center gap-3 border-b border-white/8 px-4 py-2.5 text-[12.5px] last:border-b-0">
      <span
        aria-hidden
        className={`block h-2 w-2 rounded-full ${dotClass(tone)}`}
      />
      <div className="min-w-0 truncate text-white/80">
        <span className="font-semibold text-white/95">{run.kind}</span>{" "}
        <span className="text-white/55">
          {run.success
            ? `done in ${formatDurationMs(run.duration_ms)}`
            : `${run.error_class ?? "error"} after ${formatDurationMs(run.duration_ms)}`}
        </span>
      </div>
      <span className="text-[11.5px] text-white/45">
        {formatRelativeAgo(run.finished_at_ms, nowMs)}
      </span>
    </div>
  );
}

// ─── Failed jobs panel ─────────────────────────────────────────────────

function FailedJobsCard({
  failed,
  nowMs,
}: {
  failed: ActivityFailedJob[];
  nowMs: number;
}) {
  if (failed.length === 0) {
    return (
      <div className="rounded-lg border border-white/10 bg-white/2 px-4 py-4 text-center text-sm text-white/55">
        No failed jobs. Nothing to retry.
      </div>
    );
  }
  return (
    <div className="overflow-hidden rounded-lg border border-white/10 bg-white/2">
      <div className="flex items-baseline justify-between border-b border-white/8 px-4 py-3">
        <div>
          <div className="text-[13.5px] font-semibold text-white/95">
            Failed jobs · last 50
          </div>
          <div className="text-xs text-white/55">
            Dead-letter rows. Grouped by error class.
          </div>
        </div>
      </div>
      {failed.map((j) => (
        <FailedRow key={j.id} job={j} nowMs={nowMs} />
      ))}
    </div>
  );
}

function FailedRow({ job, nowMs }: { job: ActivityFailedJob; nowMs: number }) {
  return (
    <div className="grid grid-cols-[1fr_200px_180px] items-start gap-4 border-b border-white/8 px-4 py-3 last:border-b-0">
      <div className="min-w-0">
        <div className="text-[13px] font-semibold text-white/95">
          {job.kind}
          <span className="ml-2 font-mono text-[11px] text-white/40">
            #{job.id}
          </span>
        </div>
        {job.last_error && (
          <div className="mt-0.5 truncate font-mono text-[11.5px] text-red-300/80">
            {job.last_error}
          </div>
        )}
      </div>
      <div>
        {job.error_class ? (
          <Pill tone={errorClassTone(job.error_class)} dot>
            {prettyErrorClass(job.error_class)}
          </Pill>
        ) : (
          <Pill tone="muted">unknown</Pill>
        )}
      </div>
      <div className="text-[11.5px] text-white/45">
        {job.finished_at_ms
          ? formatRelativeAgo(job.finished_at_ms, nowMs)
          : "—"}
      </div>
    </div>
  );
}

function errorClassTone(cls: string): PillTone {
  switch (cls) {
    case "external_rate_limit":
      return "warn";
    case "external_auth":
    case "permanent":
      return "bad";
    case "timeout":
      return "warn";
    default:
      return "muted";
  }
}

function prettyErrorClass(cls: string): string {
  switch (cls) {
    case "external_rate_limit":
      return "Rate limited";
    case "external_auth":
      return "Auth failure";
    case "timeout":
      return "Timeout";
    case "permanent":
      return "Permanent";
    case "transient":
      return "Transient";
    default:
      return cls;
  }
}

// ─── Helpers ───────────────────────────────────────────────────────────

function dotClass(tone: PillTone): string {
  switch (tone) {
    case "ok":
      return "bg-emerald-400";
    case "warn":
      return "bg-amber-400";
    case "bad":
      return "bg-red-400";
    case "info":
      return "bg-blue-400";
    case "accent":
      return "bg-accent";
    default:
      return "bg-white/40";
  }
}
