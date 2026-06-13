"use client";

/// Activity screen (Screen 2 from `docs/pipelines/tasks-ui.html`),
/// styled in the console design language (`cf-*`) to match the
/// redesign mockup: per-kind status cards up top, a "Recent runs"
/// table, the per-kind concurrency editor, and the failed-jobs
/// (dead-letter) panel. All powered by `/admin/tasks/activity` +
/// `/summary` with 5s polling.

import { useCallback, useEffect, useState } from "react";

import {
  admin as adminApi,
  friendlyErrorMessage,
  type ActivityFailedJob,
  type ActivityKindHealth,
  type ActivityRecentRun,
  type TasksActivityResponse,
} from "@/lib/chimpflix-api";
import {
  formatDurationMs,
  formatRelativeAgo,
} from "@/lib/relative-time";
import { DEFAULT_PAGE_SIZE, Pagination, SaveBar } from "./ui";

interface Props {
  initialActivity: TasksActivityResponse;
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
/// malformed payload by returning an empty map.
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
  initialNowMs,
  initialKindConcurrency,
}: Props) {
  const [activity, setActivity] = useState(initialActivity);
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
      const [activityRes, settingsRes] = await Promise.all([
        adminApi.tasks.activity(),
        adminApi.settings.get(),
      ]);
      setActivity(activityRes);
      setNowMs(Date.now());
      // Re-sync capBaseline from the server on each poll tick so that a
      // concurrent admin save in another session is reflected here. Skip
      // the update when the operator has unsaved local edits (dirty > 0)
      // to avoid silently discarding in-progress changes.
      setCapBaseline((prevBaseline) => {
        setCapOverrides((prevOverrides) => {
          if (countDirtyOverrides(prevBaseline, prevOverrides) > 0) {
            return prevOverrides; // leave in-progress edits alone
          }
          const fresh = parseOverrides(settingsRes.settings.job_kind_concurrency);
          return fresh;
        });
        return parseOverrides(settingsRes.settings.job_kind_concurrency);
      });
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

      {/* ── per-kind status cards ─────────────────────────────────── */}
      <PerKindCards rows={activity.per_kind} nowMs={nowMs} />

      {/* ── recent runs ───────────────────────────────────────────── */}
      <RecentRunsCard runs={activity.recent_runs} nowMs={nowMs} />

      {/* ── per-kind health (richer production view) ──────────────── */}
      <PerKindHealthCard rows={activity.per_kind} />

      {/* ── per-kind concurrency editor ───────────────────────────── */}
      <ConcurrencyEditorCard
        rows={activity.per_kind}
        overrides={capOverrides}
        onChange={setCapOverrides}
      />

      {/* ── dead letter / failed jobs ─────────────────────────────── */}
      <FailedJobsCard failed={activity.failed} nowMs={nowMs} />

      <SaveBar
        dirtyCount={countDirtyOverrides(capBaseline, capOverrides)}
        summary="per-kind concurrency caps"
        onDiscard={() => setCapOverrides(capBaseline)}
        onSave={async () => {
          // Send only kinds that diverge from the registry default.
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
/// otherwise the registry default.
function effectiveCap(
  row: ActivityKindHealth,
  overrides: Record<string, number>,
): number {
  return overrides[row.kind] ?? row.default_concurrency;
}

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

// ─── Per-kind status cards (mockup top row) ─────────────────────────────

/// Three-up grid of the busiest / most-interesting kinds, mirroring
/// the mockup: a status pill (working / failing / idle) plus the live
/// counters. Shows every active kind, not just three — the grid wraps.
function PerKindCards({
  rows,
  nowMs,
}: {
  rows: ActivityKindHealth[];
  nowMs: number;
}) {
  // Surface kinds that are doing something (in-flight, queued, or
  // recently erroring) first; fall back to all rows. Keeps the
  // headline grid focused on what's live like the mockup.
  const interesting = rows.filter(
    (r) => r.in_flight > 0 || r.queue_depth > 0 || r.recent_errors > 0,
  );
  const shown = interesting.length > 0 ? interesting : rows;
  if (shown.length === 0) return null;
  return (
    <div className="cf-grid cf-c3" style={{ marginBottom: 18 }}>
      {shown.map((r) => (
        <PerKindCard key={r.kind} row={r} nowMs={nowMs} />
      ))}
    </div>
  );
}

function PerKindCard({
  row,
  nowMs,
}: {
  row: ActivityKindHealth;
  nowMs: number;
}) {
  void nowMs;
  const failing = row.recent_errors > 0;
  const working = row.in_flight > 0;
  return (
    <div className="cf-card" style={{ marginBottom: 0 }}>
      <div className="cf-card-body cf-pad">
        <div className="cf-flex cf-between">
          <b>{row.kind}</b>
          {failing ? (
            <span className="cf-pill cf-err" style={{ padding: "1px 7px" }}>
              <span className="cf-dot" />
              failing
            </span>
          ) : working ? (
            <span className="cf-pill cf-info" style={{ padding: "1px 7px" }}>
              <span className="cf-dot" />
              working
            </span>
          ) : (
            <span className="cf-pill" style={{ padding: "1px 7px" }}>
              <span className="cf-dot" style={{ background: "var(--ghost)" }} />
              idle
            </span>
          )}
        </div>
        <div className="cf-muted" style={{ fontSize: 12, marginTop: 8 }}>
          {row.queue_depth} queued · {row.in_flight} running
          <br />
          {failing
            ? `${row.recent_errors} error${row.recent_errors === 1 ? "" : "s"} recently`
            : `${row.jobs_per_minute.toFixed(1)} jobs/min · p95 ${
                row.p95_duration_ms == null
                  ? "—"
                  : formatDurationMs(row.p95_duration_ms)
              }`}
        </div>
      </div>
    </div>
  );
}

// ─── Recent runs (mockup table) ─────────────────────────────────────────

function RecentRunsCard({
  runs,
  nowMs,
}: {
  runs: ActivityRecentRun[];
  nowMs: number;
}) {
  // Client-side pagination — the server caps at 200 entries (the ring
  // buffer's natural upper bound) so we slice the visible chunk locally.
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);
  const totalPages = Math.max(1, Math.ceil(runs.length / pageSize));
  const effectivePage = Math.min(page, totalPages);
  const start = (effectivePage - 1) * pageSize;
  const slice = runs.slice(start, start + pageSize);

  return (
    <div className="cf-card">
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">Recent runs</div>
          <div className="cf-sub">Job completions, newest first.</div>
        </div>
        <div className="cf-head-aside">
          <span className="cf-pill cf-ok">
            <span className="cf-dot" />
            Live
          </span>
        </div>
      </div>
      {runs.length === 0 ? (
        <div className="cf-card-body cf-pad cf-center cf-faint">
          No completed runs since process started.
        </div>
      ) : (
        <>
          <table className="cf-table">
            <thead>
              <tr>
                <th>When</th>
                <th>Kind</th>
                <th>Duration</th>
                <th>Result</th>
              </tr>
            </thead>
            <tbody>
              {slice.map((r) => (
                <tr key={`${r.kind}-${r.finished_at_ms}`}>
                  <td className="cf-faint">
                    {formatRelativeAgo(r.finished_at_ms, nowMs)}
                  </td>
                  <td className="cf-mono">{r.kind}</td>
                  <td className="cf-mono">{formatDurationMs(r.duration_ms)}</td>
                  <td>
                    {r.success ? (
                      <span
                        className="cf-pill cf-ok"
                        style={{ padding: "1px 7px" }}
                      >
                        <span className="cf-dot" />
                        ok
                      </span>
                    ) : (
                      <span
                        className={`cf-pill ${
                          r.error_class === "external_rate_limit"
                            ? "cf-warn"
                            : "cf-err"
                        }`}
                        style={{ padding: "1px 7px" }}
                      >
                        <span className="cf-dot" />
                        {r.error_class === "external_rate_limit"
                          ? "rate-limited"
                          : "failed"}
                      </span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {runs.length > pageSize && (
            <div className="cf-card-body" style={{ paddingBottom: 14 }}>
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

// ─── Per-kind health (richer production table) ──────────────────────────

function PerKindHealthCard({ rows }: { rows: ActivityKindHealth[] }) {
  return (
    <div className="cf-card">
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">Per-kind health</div>
          <div className="cf-sub">
            Live counters · p95 from in-memory ring (last 100 runs).
          </div>
        </div>
      </div>
      {rows.length === 0 ? (
        <div className="cf-card-body cf-pad cf-center cf-faint">
          No kinds active.
        </div>
      ) : (
        <table className="cf-table">
          <thead>
            <tr>
              <th>Kind</th>
              <th className="cf-num">Queue</th>
              <th className="cf-num">In-flight</th>
              <th className="cf-num">Per min</th>
              <th className="cf-num">p95</th>
              <th className="cf-num">ETA</th>
              <th className="cf-num">Errors</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <PerKindHealthRow key={r.kind} row={r} />
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function PerKindHealthRow({ row }: { row: ActivityKindHealth }) {
  return (
    <tr>
      <td>
        {row.display_name}
        <span
          className="cf-mono cf-faint"
          style={{ marginLeft: 8, fontSize: 11 }}
        >
          {row.kind}
        </span>
      </td>
      <td className="cf-num cf-mono">{row.queue_depth}</td>
      <td className="cf-num cf-mono">{row.in_flight}</td>
      <td className="cf-num cf-mono">{row.jobs_per_minute.toFixed(1)}</td>
      <td className="cf-num cf-mono">
        {row.p95_duration_ms == null ? "—" : formatDurationMs(row.p95_duration_ms)}
      </td>
      <td
        className="cf-num cf-mono"
        title={
          row.eta_seconds_remaining == null
            ? row.queue_depth === 0
              ? "Queue empty — nothing to drain."
              : "Not enough run history yet to estimate (need 5+ successful runs)."
            : `${row.queue_depth} queued × p95 ÷ effective concurrency. Coarse estimate; assumes current throughput.`
        }
      >
        {formatEta(row.eta_seconds_remaining)}
      </td>
      <td
        className="cf-num cf-mono"
        style={{ color: row.recent_errors > 0 ? "#fca5a5" : undefined }}
      >
        {row.recent_errors}
      </td>
    </tr>
  );
}

/// Render a wall-clock ETA as a short human label.
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

// ─── Per-kind concurrency editor ────────────────────────────────────────

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
  // by `KindLimiter`.
  const eligible = rows.filter((r) => r.default_concurrency >= 1);
  if (eligible.length === 0) return null;

  function setCapFor(kind: string, value: number, fallback: number) {
    const next = { ...overrides };
    if (value === fallback) {
      delete next[kind];
    } else {
      next[kind] = value;
    }
    onChange(next);
  }

  return (
    <div className="cf-card">
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">Per-kind concurrency</div>
          <div className="cf-sub">
            Raise the cap on a kind to let more of it run in parallel. Defaults
            are conservative — bump CPU-heavy kinds (markers / preview) on hefty
            boxes. Saved changes apply live; no restart required.
          </div>
        </div>
      </div>
      <table className="cf-table">
        <thead>
          <tr>
            <th>Kind</th>
            <th className="cf-num">Default</th>
            <th className="cf-num">Cap</th>
          </tr>
        </thead>
        <tbody>
          {eligible.map((r) => {
            const value = effectiveCap(r, overrides);
            const isOverride = overrides[r.kind] !== undefined;
            return (
              <tr key={r.kind}>
                <td>
                  {r.display_name}
                  <span
                    className="cf-mono cf-faint"
                    style={{ marginLeft: 8, fontSize: 11 }}
                  >
                    {r.kind}
                  </span>
                </td>
                <td className="cf-num cf-mono">
                  {r.default_concurrency}
                  {isOverride && (
                    <span
                      className="cf-tag"
                      style={{ marginLeft: 8, color: "var(--warn)" }}
                    >
                      overridden
                    </span>
                  )}
                </td>
                <td className="cf-num">
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
                    className="cf-input"
                    style={{ width: 72, textAlign: "right" }}
                    aria-label={`Concurrency cap for ${r.display_name}`}
                  />
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

// ─── Failed jobs / dead-letter panel ────────────────────────────────────

function FailedJobsCard({
  failed,
  nowMs,
}: {
  failed: ActivityFailedJob[];
  nowMs: number;
}) {
  if (failed.length === 0) {
    return (
      <div className="cf-card" style={{ marginBottom: 0 }}>
        <div className="cf-card-body cf-pad cf-center cf-faint">
          No failed jobs. Nothing to retry.
        </div>
      </div>
    );
  }
  return (
    <div className="cf-card" style={{ marginBottom: 0 }}>
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">Dead letter · last 50</div>
          <div className="cf-sub">
            Jobs that exhausted retries — resurrect from the Queue tab.
          </div>
        </div>
      </div>
      <table className="cf-table">
        <thead>
          <tr>
            <th>Job</th>
            <th>Kind</th>
            <th>Error</th>
            <th>When</th>
          </tr>
        </thead>
        <tbody>
          {failed.map((j) => (
            <tr key={j.id}>
              <td className="cf-mono">#{j.id}</td>
              <td className="cf-mono">{j.kind}</td>
              <td>
                <div
                  className="cf-mono"
                  style={{ color: "#fca5a5", fontSize: 12 }}
                >
                  {j.last_error ?? "—"}
                </div>
                {j.error_class && (
                  <span
                    className={`cf-pill ${errorClassPillTone(j.error_class)}`}
                    style={{ marginTop: 4, padding: "1px 7px" }}
                  >
                    <span className="cf-dot" />
                    {prettyErrorClass(j.error_class)}
                  </span>
                )}
              </td>
              <td className="cf-faint">
                {j.finished_at_ms
                  ? formatRelativeAgo(j.finished_at_ms, nowMs)
                  : "—"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function errorClassPillTone(cls: string): string {
  switch (cls) {
    case "external_rate_limit":
    case "timeout":
      return "cf-warn";
    case "external_auth":
    case "permanent":
      return "cf-err";
    default:
      return "";
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
