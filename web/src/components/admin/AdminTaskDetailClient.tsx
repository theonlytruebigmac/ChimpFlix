"use client";

/// Detail screen (Screen 3 from `docs/pipelines/tasks-ui.html`),
/// styled in the console design language (`cf-*`) to match the
/// redesign. Per-kind drill-in: a stat strip, an editable gate +
/// schedule pair, a 30-day history chart, and the recent-runs table.
/// Backed by `/admin/tasks/kind/{name}` with 5s polling for the live
/// counters.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Link from "next/link";

import {
  admin as adminApi,
  friendlyErrorMessage,
  type ActivityRecentRun,
  type KindDetailDailyMetrics,
  type KindDetailResponse,
  type TaskFrequency,
  type TaskMode,
} from "@/lib/chimpflix-api";
import {
  formatDurationMs,
  formatRelativeAgo,
  formatRelativeFuture,
} from "@/lib/relative-time";
import { TOAST_DISMISS_SHORT_MS } from "@/lib/toast";

/// Frequency dropdown options — superset of what `prettyFrequency()`
/// can label. `custom` is intentionally omitted: editing raw cron
/// expressions belongs in a low-level config tool.
const FREQUENCY_OPTIONS: TaskFrequency[] = [
  "manual",
  "hourly",
  "every_3_hours",
  "every_6_hours",
  "every_12_hours",
  "daily",
  "every_3_days",
  "weekly",
  "monthly",
  "on_change",
];

interface Props {
  initial: KindDetailResponse;
  /// `Date.now()` snapshot from the server fetch (SSR-hydration
  /// stability — see AdminTasksOverviewClient).
  initialNowMs: number;
}

const REFRESH_MS = 5_000;

export function AdminTaskDetailClient({ initial, initialNowMs }: Props) {
  const [detail, setDetail] = useState(initial);
  const [nowMs, setNowMs] = useState(initialNowMs);
  const [busy, setBusy] = useState(false);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedNotice, setSavedNotice] = useState<string | null>(null);
  // Stored so we can cancel the toast on unmount (avoids setState on
  // an unmounted component if the user navigates away before it fires).
  const savedNoticeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (savedNoticeTimer.current != null) clearTimeout(savedNoticeTimer.current);
    };
  }, []);

  // Capture the kind name once at mount so polling doesn't tear down +
  // recreate the interval each time `detail` updates.
  const kindName = initial.name;

  const refresh = useCallback(async () => {
    try {
      const next = await adminApi.tasks.detail(kindName);
      setDetail(next);
      setNowMs(Date.now());
      setError(null);
    } catch (e) {
      setError(friendlyErrorMessage(e));
    }
  }, [kindName]);

  useEffect(() => {
    const id = setInterval(refresh, REFRESH_MS);
    return () => clearInterval(id);
  }, [refresh]);

  const toggleGate = useCallback(
    async (next: boolean) => {
      if (detail.gate.locked) return;
      setBusy(true);
      setDetail((prev) => ({
        ...prev,
        gate: { ...prev.gate, enabled: next },
      }));
      try {
        await adminApi.tasks.setGate(kindName, next);
        await refresh();
      } catch (e) {
        setDetail((prev) => ({
          ...prev,
          gate: { ...prev.gate, enabled: !next },
        }));
        setError(friendlyErrorMessage(e));
      } finally {
        setBusy(false);
      }
    },
    [detail.gate.locked, kindName, refresh],
  );

  const runNow = useCallback(async () => {
    setRunning(true);
    setError(null);
    setSavedNotice(null);
    try {
      await adminApi.tasks.runKindNow(kindName);
      // Give the scheduler a beat to mark the row running before
      // refresh; otherwise the badge flips back to idle.
      await new Promise((r) => setTimeout(r, 600));
      await refresh();
    } catch (e) {
      setError(friendlyErrorMessage(e));
    } finally {
      setRunning(false);
    }
  }, [kindName, refresh]);

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
      {savedNotice && (
        <div className="cf-banner cf-ok">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M20 6L9 17l-5-5" />
          </svg>
          <div>{savedNotice}</div>
        </div>
      )}

      {/* ── header ────────────────────────────────────────────────── */}
      <div
        className="cf-flex cf-between cf-wrap cf-gap12"
        style={{ marginBottom: 6 }}
      >
        <div className="cf-flex cf-wrap cf-gap8">
          <span style={{ fontSize: 22, fontWeight: 800, letterSpacing: "-0.02em" }}>
            {detail.display_name}
          </span>
          <ModeBadge mode={detail.mode} />
          {detail.gate.locked ? (
            <span className="cf-pill cf-ok">
              <span className="cf-dot" />
              Always on
            </span>
          ) : (
            <span className={`cf-pill${detail.gate.enabled ? " cf-ok" : ""}`}>
              <span
                className="cf-dot"
                style={detail.gate.enabled ? undefined : { background: "var(--ghost)" }}
              />
              {detail.gate.enabled ? "Enabled" : "Disabled"}
            </span>
          )}
          {detail.live.in_flight > 0 && (
            <span className="cf-pill cf-info">
              <span className="cf-dot" />
              {detail.live.in_flight} running
            </span>
          )}
        </div>
        <div className="cf-flex cf-gap8">
          {detail.schedule && (
            <button
              type="button"
              className="cf-btn cf-sm"
              onClick={runNow}
              disabled={running || detail.live.in_flight > 0}
              title={
                detail.live.in_flight > 0
                  ? "A run is already in flight"
                  : "Trigger this task once now"
              }
            >
              {running ? "Starting…" : "Run now"}
            </button>
          )}
          <Link className="cf-btn cf-ghost cf-sm" href="/settings/admin/tasks">
            ← Back to tasks
          </Link>
        </div>
      </div>

      <p className="cf-mono cf-faint" style={{ fontSize: 12, marginBottom: 18 }}>
        {detail.name} · {detail.scope.replace("_", " ")}
      </p>

      <StatStrip detail={detail} nowMs={nowMs} />

      <div className="cf-grid cf-c2">
        <GateCard detail={detail} busy={busy} onToggle={toggleGate} />
        <ScheduleCard
          detail={detail}
          kindName={kindName}
          nowMs={nowMs}
          onSaved={(next) => {
            setDetail(next);
            setNowMs(Date.now());
            setSavedNotice("Schedule saved.");
            setError(null);
            if (savedNoticeTimer.current != null) clearTimeout(savedNoticeTimer.current);
            savedNoticeTimer.current = setTimeout(() => setSavedNotice(null), TOAST_DISMISS_SHORT_MS);
          }}
          onError={(msg) => {
            setError(msg);
            setSavedNotice(null);
          }}
        />
      </div>

      <HistoryCard history={detail.history} />

      <RecentRunsCard runs={detail.recent_runs} />
    </div>
  );
}

// ─── Pieces ────────────────────────────────────────────────────────────

function ModeBadge({ mode }: { mode: TaskMode }) {
  if (mode === "automatic") {
    return (
      <span className="cf-pill cf-info">
        <span className="cf-dot" />
        Automatic
      </span>
    );
  }
  if (mode === "gated") {
    return (
      <span className="cf-pill cf-warn">
        <span className="cf-dot" />
        Gated
      </span>
    );
  }
  return (
    <span className="cf-pill">
      <span className="cf-dot" style={{ background: "var(--ghost)" }} />
      Periodic
    </span>
  );
}

function StatStrip({
  detail,
  nowMs,
}: {
  detail: KindDetailResponse;
  nowMs: number;
}) {
  // Aggregate the 30-day history into the headline numbers.
  const histTotals = detail.history.reduce(
    (acc, d) => ({
      success: acc.success + d.success_count,
      failure: acc.failure + d.failure_count,
      targets: acc.targets + d.targets_processed,
    }),
    { success: 0, failure: 0, targets: 0 },
  );
  const histTotal = histTotals.success + histTotals.failure;

  // Ring-buffer fallback for Automatic kinds with no schedule row and
  // no daily rollup yet.
  const ringSuccess = detail.recent_runs.filter((r) => r.success).length;
  const ringFailure = detail.recent_runs.length - ringSuccess;
  const ringTotal = detail.recent_runs.length;
  const ringNewest = detail.recent_runs[0]?.finished_at_ms ?? null;

  const usingRing = histTotal === 0 && ringTotal > 0;

  const lastRunMs = detail.schedule?.last_at ?? ringNewest;
  const totalRuns = histTotal > 0 ? histTotal : ringTotal;
  const totalTargets = histTotal > 0 ? histTotals.targets : ringTotal;
  const failureCount = histTotal > 0 ? histTotals.failure : ringFailure;
  const successCount = histTotal > 0 ? histTotals.success : ringSuccess;
  const successRate = totalRuns === 0 ? null : (successCount / totalRuns) * 100;

  // Guard `next_at === 0` (epoch 1970 — sweep enabled but no first run
  // computed yet); "in 56y" looks like a bug.
  const hasNext =
    detail.schedule?.enabled && (detail.schedule?.next_at ?? 0) > 0;

  return (
    <div
      className="cf-grid"
      style={{ gridTemplateColumns: "repeat(5, 1fr)", marginBottom: 18 }}
    >
      <Stat
        label="Last run"
        value={lastRunMs ? formatRelativeAgo(lastRunMs, nowMs) : "—"}
        sub={lastRunMs ? toIsoDate(lastRunMs) : ""}
      />
      <Stat
        label="Next sweep"
        value={hasNext ? formatRelativeFuture(detail.schedule!.next_at, nowMs) : "—"}
        sub={
          hasNext
            ? toIsoDate(detail.schedule!.next_at)
            : detail.schedule
              ? detail.schedule.enabled
                ? "awaiting first run"
                : "sweep disabled"
              : "on-add only"
        }
      />
      <Stat
        label={usingRing ? "Runs (recent)" : "Runs (30d)"}
        value={totalRuns.toLocaleString()}
        sub={
          usingRing
            ? "from in-memory ring · rollup pending"
            : `${totalTargets.toLocaleString()} targets processed`
        }
      />
      <Stat
        label="Success rate"
        value={successRate == null ? "—" : `${successRate.toFixed(1)}%`}
        sub={
          successRate == null
            ? "no runs yet"
            : `${failureCount} of ${totalRuns} failed`
        }
      />
      <Stat
        label="p95 duration"
        value={
          detail.p95_duration_ms == null
            ? "—"
            : formatDurationMs(detail.p95_duration_ms)
        }
        sub="per run (recent ring)"
      />
    </div>
  );
}

function Stat({
  label,
  value,
  sub,
}: {
  label: string;
  value: string;
  sub?: string;
}) {
  return (
    <div className="cf-stat" style={{ padding: 14 }}>
      <div className="cf-stat-top">{label}</div>
      <div className="cf-stat-val" style={{ fontSize: 20 }}>
        {value}
      </div>
      {sub && <div className="cf-stat-meta">{sub}</div>}
    </div>
  );
}

function GateCard({
  detail,
  busy,
  onToggle,
}: {
  detail: KindDetailResponse;
  busy: boolean;
  onToggle: (next: boolean) => Promise<void>;
}) {
  const settingKey = detail.gate.setting_key;
  return (
    <div className="cf-card">
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">Gate</div>
          <div className="cf-sub">
            Both the on-add pipeline and the safety-net sweep consult this gate.
          </div>
        </div>
        <div className="cf-head-aside">
          {detail.gate.locked ? (
            <span className="cf-pill cf-ok">
              <span className="cf-dot" />
              Always on
            </span>
          ) : (
            <button
              type="button"
              role="switch"
              aria-checked={detail.gate.enabled}
              aria-label="Gate enabled"
              disabled={busy}
              onClick={() => onToggle(!detail.gate.enabled)}
              className={`cf-switch${detail.gate.enabled ? " cf-on" : ""}`}
            />
          )}
        </div>
      </div>
      <div className="cf-card-body cf-pad cf-muted" style={{ fontSize: 12.5 }}>
        {settingKey ? (
          <p>
            Setting key:{" "}
            <code className="cf-mono cf-tag">{settingKey}</code> — toggling here
            is equivalent to flipping it on{" "}
            <code className="cf-mono cf-tag">/admin/settings</code>.
          </p>
        ) : (
          <p>
            Automatic kinds run on every new file/item; the only switch is
            removing the underlying dependency (e.g. clearing the TMDB key for{" "}
            {detail.display_name}).
          </p>
        )}
      </div>
    </div>
  );
}

function ScheduleCard({
  detail,
  kindName,
  nowMs,
  onSaved,
  onError,
}: {
  detail: KindDetailResponse;
  kindName: string;
  nowMs: number;
  onSaved: (next: KindDetailResponse) => void;
  onError: (msg: string) => void;
}) {
  // The "trigger" case (no schedule row) is informational — these are
  // Automatic kinds that fire only on the on-add event.
  if (!detail.schedule) {
    return (
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Trigger</div>
            <div className="cf-sub">On-add only — no sweep cron.</div>
          </div>
        </div>
        <div className="cf-card-body cf-pad cf-muted" style={{ fontSize: 12.5 }}>
          This kind runs only when the scanner emits a relevant event.
          There&apos;s no scheduled safety-net to configure.
        </div>
      </div>
    );
  }

  // Mount the editor with a key derived from the schedule signature we
  // treat as "baseline" — Save / navigation remounts with new initial
  // values, no useEffect syncing, and 5s polls don't clobber edits.
  const key = `${kindName}|${detail.schedule.frequency}|${detail.schedule.enabled}|${detail.schedule.requires_maintenance_window}`;
  return (
    <ScheduleEditor
      key={key}
      schedule={detail.schedule}
      kindName={kindName}
      nowMs={nowMs}
      onSaved={onSaved}
      onError={onError}
    />
  );
}

function ScheduleEditor({
  schedule,
  kindName,
  nowMs,
  onSaved,
  onError,
}: {
  schedule: NonNullable<KindDetailResponse["schedule"]>;
  kindName: string;
  nowMs: number;
  onSaved: (next: KindDetailResponse) => void;
  onError: (msg: string) => void;
}) {
  const initialFrequency = schedule.frequency as TaskFrequency;
  const initialEnabled = schedule.enabled;
  const initialWindow = schedule.requires_maintenance_window;

  const [frequency, setFrequency] = useState<TaskFrequency>(initialFrequency);
  const [enabled, setEnabled] = useState<boolean>(initialEnabled);
  const [windowSnap, setWindowSnap] = useState<boolean>(initialWindow);
  const [saving, setSaving] = useState(false);

  const dirty = useMemo(
    () =>
      frequency !== initialFrequency ||
      enabled !== initialEnabled ||
      windowSnap !== initialWindow,
    [frequency, enabled, windowSnap, initialFrequency, initialEnabled, initialWindow],
  );

  const showCustomOption = initialFrequency === "custom";

  async function save() {
    setSaving(true);
    try {
      const next = await adminApi.tasks.updateKindSchedule(kindName, {
        frequency,
        enabled,
        requires_maintenance_window: windowSnap,
      });
      onSaved(next);
    } catch (e) {
      onError(friendlyErrorMessage(e));
    } finally {
      setSaving(false);
    }
  }

  function reset() {
    setFrequency(initialFrequency);
    setEnabled(initialEnabled);
    setWindowSnap(initialWindow);
  }

  return (
    <div className="cf-card">
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">Schedule</div>
          <div className="cf-sub">
            When the safety-net sweep fires. On-add events ignore this — they
            fire as files appear.
          </div>
        </div>
      </div>
      <div className="cf-card-body">
        <div className="cf-row">
          <div className="cf-row-main">
            <div className="cf-row-label">Frequency</div>
          </div>
          <div className="cf-row-control">
            <select
              className="cf-select cf-w-auto"
              value={frequency}
              onChange={(e) => setFrequency(e.target.value as TaskFrequency)}
              disabled={saving}
            >
              {FREQUENCY_OPTIONS.map((f) => (
                <option key={f} value={f}>
                  {prettyFrequency(f)}
                </option>
              ))}
              {showCustomOption && (
                <option value="custom">Custom cron (advanced)</option>
              )}
            </select>
          </div>
        </div>
        <div className="cf-row">
          <div className="cf-row-main">
            <div className="cf-row-label">Sweep enabled</div>
          </div>
          <div className="cf-row-control">
            <Toggle
              checked={enabled}
              disabled={saving}
              onChange={setEnabled}
              ariaLabel="Sweep enabled"
            />
          </div>
        </div>
        <div className="cf-row">
          <div className="cf-row-main">
            <div className="cf-row-label">Snap to maintenance window</div>
            <div className="cf-row-help">
              Defers heavy sweeps into the configured low-traffic window so they
              don&rsquo;t compete with playback.
            </div>
          </div>
          <div className="cf-row-control">
            <Toggle
              checked={windowSnap}
              disabled={saving}
              onChange={setWindowSnap}
              ariaLabel="Snap to maintenance window"
            />
          </div>
        </div>
        <div className="cf-row cf-col">
          <div className="cf-faint" style={{ fontSize: 11.5 }}>
            {schedule.next_at > 0 && (
              <div>
                Next run: {formatRelativeFuture(schedule.next_at, nowMs)} (
                {toIsoDate(schedule.next_at)})
              </div>
            )}
            {schedule.last_at != null && (
              <div>
                Last run: {formatRelativeAgo(schedule.last_at, nowMs)} (
                {toIsoDate(schedule.last_at)}) — {schedule.last_status}
              </div>
            )}
          </div>
          <div
            className="cf-flex cf-gap8"
            style={{ justifyContent: "flex-end", width: "100%" }}
          >
            <button
              type="button"
              className="cf-btn cf-ghost cf-sm"
              onClick={reset}
              disabled={!dirty || saving}
            >
              Reset
            </button>
            <button
              type="button"
              className="cf-btn cf-primary cf-sm"
              onClick={save}
              disabled={!dirty || saving}
            >
              {saving ? "Saving…" : "Save schedule"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function Toggle({
  checked,
  disabled,
  onChange,
  ariaLabel,
}: {
  checked: boolean;
  disabled?: boolean;
  onChange: (next: boolean) => void;
  ariaLabel: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`cf-switch${checked ? " cf-on" : ""}`}
    />
  );
}

// ─── 30-day history chart ──────────────────────────────────────────────

function HistoryCard({ history }: { history: KindDetailDailyMetrics[] }) {
  return (
    <div className="cf-card">
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">History — 30 days</div>
          <div className="cf-sub">
            Targets processed per day · failures overlaid in red.
          </div>
        </div>
      </div>
      <div className="cf-card-body cf-pad">
        {history.length === 0 ? (
          <div className="cf-center cf-faint" style={{ padding: "32px 16px" }}>
            No rollup data yet. The daily rollup task runs at 02:00; the chart
            fills in as it completes its first runs.
          </div>
        ) : (
          <HistoryChart history={history} />
        )}
      </div>
    </div>
  );
}

function HistoryChart({ history }: { history: KindDetailDailyMetrics[] }) {
  const max = Math.max(
    1,
    ...history.map((d) => d.success_count + d.failure_count),
  );
  const barWidth = 580 / Math.max(history.length, 1);
  return (
    <svg
      viewBox="0 0 600 100"
      preserveAspectRatio="none"
      style={{ width: "100%", height: 128 }}
    >
      <line x1="0" y1="25" x2="600" y2="25" stroke="rgba(255,255,255,0.05)" strokeWidth="1" />
      <line x1="0" y1="50" x2="600" y2="50" stroke="rgba(255,255,255,0.05)" strokeWidth="1" />
      <line x1="0" y1="75" x2="600" y2="75" stroke="rgba(255,255,255,0.05)" strokeWidth="1" />
      {history.map((d, i) => {
        const total = d.success_count + d.failure_count;
        const h = Math.round((total / max) * 90);
        const x = 10 + i * barWidth;
        const y = 100 - h;
        return (
          <g key={d.day_ms}>
            <rect
              x={x}
              y={y}
              width={Math.max(barWidth - 4, 2)}
              height={h}
              fill="#34d399"
              opacity={0.9}
            />
            {d.failure_count > 0 && (
              <circle cx={x + barWidth / 2} cy={Math.max(y - 5, 6)} r={3.5} fill="#f87171" />
            )}
          </g>
        );
      })}
    </svg>
  );
}

// ─── Recent runs ───────────────────────────────────────────────────────

function RecentRunsCard({ runs }: { runs: ActivityRecentRun[] }) {
  if (runs.length === 0) {
    return null;
  }
  return (
    <div className="cf-card" style={{ marginBottom: 0 }}>
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">Recent runs</div>
          <div className="cf-sub">In-memory ring buffer · resets on restart.</div>
        </div>
      </div>
      <table className="cf-table">
        <thead>
          <tr>
            <th>Finished</th>
            <th>Duration</th>
            <th>Status</th>
            <th>Notes</th>
          </tr>
        </thead>
        <tbody>
          {runs.map((r) => (
            <tr key={r.finished_at_ms}>
              <td>{toIsoDateTime(r.finished_at_ms)}</td>
              <td className="cf-mono">{formatDurationMs(r.duration_ms)}</td>
              <td>
                {r.success ? (
                  <span className="cf-pill cf-ok" style={{ padding: "1px 7px" }}>
                    <span className="cf-dot" />
                    ok
                  </span>
                ) : (
                  <span className="cf-pill cf-err" style={{ padding: "1px 7px" }}>
                    <span className="cf-dot" />
                    {prettyErrorClass(r.error_class)}
                  </span>
                )}
              </td>
              <td className="cf-faint">
                {r.success ? "—" : (r.error_class ?? "unclassified")}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// ─── Helpers ─────────────────────────────────────────────────────────────

function prettyFrequency(f: string): string {
  switch (f) {
    case "manual":
      return "Manual only";
    case "hourly":
      return "Every hour";
    case "every_3_hours":
      return "Every 3 hours";
    case "every_6_hours":
      return "Every 6 hours";
    case "every_12_hours":
      return "Every 12 hours";
    case "daily":
      return "Daily";
    case "every_3_days":
      return "Every 3 days";
    case "weekly":
      return "Weekly";
    case "monthly":
      return "Monthly";
    case "on_change":
      return "On change";
    case "custom":
      return "Custom cron";
    default:
      return f;
  }
}

function prettyErrorClass(cls: string | null): string {
  switch (cls) {
    case "external_rate_limit":
      return "rate limited";
    case "external_auth":
      return "auth failed";
    case "timeout":
      return "timed out";
    case "permanent":
      return "permanent";
    case "transient":
      return "transient";
    default:
      return cls ?? "failed";
  }
}

function toIsoDate(ms: number): string {
  return new Date(ms).toISOString().slice(0, 10);
}

function toIsoDateTime(ms: number): string {
  return new Date(ms).toISOString().slice(0, 19).replace("T", " ");
}
