"use client";

/// Detail screen (Screen 3 from `docs/pipelines/tasks-ui.html`).
/// Per-kind drill-in: schedule + gate + live counters + recent runs
/// + 30-day history chart. Backed by `/admin/tasks/kind/{name}` with
/// 5s polling for the live counters.

import { useCallback, useEffect, useMemo, useState } from "react";
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
import { Pill } from "./ui";

/// Frequency dropdown options — superset of what `prettyFrequency()`
/// can label. `custom` is intentionally omitted: editing raw cron
/// expressions belongs in a low-level config tool, not the curated
/// scheduled-tasks surface. Rows that already have `custom` still
/// display "Custom cron" via the existing label table but switching
/// AWAY from custom requires picking one of the friendly cadences.
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

  // Capture the kind name once at mount so polling doesn't tear
  // down + recreate the interval each time `detail` updates.
  // Detail pages are scoped to a single kind for their lifetime;
  // there's no scenario where the name changes mid-render.
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
      // refresh; otherwise the badge flips back to idle and the
      // operator thinks the click was a no-op.
      await new Promise((r) => setTimeout(r, 600));
      await refresh();
    } catch (e) {
      setError(friendlyErrorMessage(e));
    } finally {
      setRunning(false);
    }
  }, [kindName, refresh]);

  return (
    <div className="space-y-6">
      {error && (
        <div className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-300">
          {error}
        </div>
      )}
      {savedNotice && (
        <div className="rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
          {savedNotice}
        </div>
      )}

      <div className="flex items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <h1 className="text-xl font-semibold tracking-tight text-white/95">
            {detail.display_name}
          </h1>
          <ModeBadge mode={detail.mode} />
          {detail.gate.locked ? (
            <Pill tone="ok" dot>
              Always on
            </Pill>
          ) : (
            <Pill tone={detail.gate.enabled ? "ok" : "muted"} dot>
              {detail.gate.enabled ? "Enabled" : "Disabled"}
            </Pill>
          )}
          {detail.live.in_flight > 0 && (
            <Pill tone="info" dot>
              {detail.live.in_flight} running
            </Pill>
          )}
        </div>
        <div className="flex items-center gap-2">
          {detail.schedule && (
            <button
              type="button"
              onClick={runNow}
              disabled={running || detail.live.in_flight > 0}
              className="rounded border border-white/20 px-3 py-1 text-xs font-medium text-white/85 transition-colors hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-50"
              title={
                detail.live.in_flight > 0
                  ? "A run is already in flight"
                  : "Trigger this task once now"
              }
            >
              {running ? "Starting…" : "Run now"}
            </button>
          )}
          <Link
            href="/settings/admin/library/scheduled-tasks"
            className="rounded border border-white/15 px-2.5 py-1 text-xs text-white/60 transition-colors hover:bg-white/5"
          >
            ← Back to tasks
          </Link>
        </div>
      </div>

      <p className="font-mono text-xs text-white/45">
        {detail.name} · {detail.scope.replace("_", " ")}
      </p>

      <StatStrip detail={detail} nowMs={nowMs} />

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <GateCard
          detail={detail}
          busy={busy}
          onToggle={toggleGate}
        />
        <ScheduleCard
          detail={detail}
          kindName={kindName}
          nowMs={nowMs}
          onSaved={(next) => {
            setDetail(next);
            setNowMs(Date.now());
            setSavedNotice("Schedule saved.");
            setError(null);
            window.setTimeout(() => setSavedNotice(null), 2500);
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
      <Pill tone="info" dot>
        Automatic
      </Pill>
    );
  }
  if (mode === "gated") {
    return (
      <Pill tone="warn" dot>
        Gated
      </Pill>
    );
  }
  return (
    <Pill tone="muted" dot>
      Periodic
    </Pill>
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

  // Ring-buffer fallback. For Automatic kinds (Preview sprite,
  // chapter thumbs, etc.) there's no `scheduled_tasks` row, so
  // `schedule.last_at` is null, and the daily rollup at 02:00
  // hasn't populated `history` yet either — the headline numbers
  // would all dash out even though the kind is actively running.
  // Use the in-memory ring buffer (`detail.recent_runs`) so a
  // freshly-deployed instance shows real activity within minutes
  // instead of waiting for the next rollup. Once rollup data
  // arrives the history values win (they're the durable record).
  const ringSuccess = detail.recent_runs.filter((r) => r.success).length;
  const ringFailure = detail.recent_runs.length - ringSuccess;
  const ringTotal = detail.recent_runs.length;
  const ringNewest = detail.recent_runs[0]?.finished_at_ms ?? null;

  const usingRing = histTotal === 0 && ringTotal > 0;

  // Effective values for the headline stats. Prefer schedule.last_at
  // for kinds that have a sweep (it's the persisted source of
  // truth across restarts); fall back to the ring's newest entry
  // for Automatic kinds.
  const lastRunMs = detail.schedule?.last_at ?? ringNewest;
  const totalRuns = histTotal > 0 ? histTotal : ringTotal;
  const totalTargets =
    histTotal > 0 ? histTotals.targets : ringTotal; // ring rows don't carry targets_processed
  const failureCount = histTotal > 0 ? histTotals.failure : ringFailure;
  const successCount = histTotal > 0 ? histTotals.success : ringSuccess;
  const successRate =
    totalRuns === 0 ? null : (successCount / totalRuns) * 100;

  // Guard `next_at === 0` (epoch 1970 — appears when the sweep is
  // enabled but the scheduler hasn't computed a first run yet).
  // Rendering "in 56y" looks like a date-handling bug.
  const hasNext =
    detail.schedule?.enabled && (detail.schedule?.next_at ?? 0) > 0;

  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
      <Stat
        label="Last run"
        value={lastRunMs ? formatRelativeAgo(lastRunMs, nowMs) : "—"}
        sub={lastRunMs ? toIsoDate(lastRunMs) : ""}
      />
      <Stat
        label="Next sweep"
        value={
          hasNext ? formatRelativeFuture(detail.schedule!.next_at, nowMs) : "—"
        }
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
    <div className="rounded-lg border border-white/10 bg-white/2 px-3 py-3">
      <div className="text-[10.5px] font-semibold uppercase tracking-[0.07em] text-white/45">
        {label}
      </div>
      <div className="mt-1 text-lg font-semibold text-white/95">{value}</div>
      {sub && <div className="mt-0.5 text-[11.5px] text-white/45">{sub}</div>}
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
    <div className="rounded-lg border border-white/10 bg-white/2">
      <div className="flex items-center justify-between border-b border-white/8 px-4 py-3">
        <div>
          <div className="text-[13.5px] font-semibold text-white/95">Gate</div>
          <div className="text-xs text-white/55">
            Both the on-add pipeline and the safety-net sweep consult this gate.
          </div>
        </div>
        {detail.gate.locked ? (
          <Pill tone="ok" dot>
            Always on
          </Pill>
        ) : (
          <button
            type="button"
            role="switch"
            aria-checked={detail.gate.enabled}
            disabled={busy}
            onClick={() => onToggle(!detail.gate.enabled)}
            className={`relative inline-flex h-5 w-10 cursor-pointer items-center rounded-full border transition-colors disabled:cursor-wait ${
              detail.gate.enabled
                ? "border-emerald-500/70 bg-emerald-500"
                : "border-white/20 bg-white/12"
            }`}
          >
            <span
              className={`block h-4 w-4 rounded-full bg-white shadow-sm transition-transform ${
                detail.gate.enabled ? "translate-x-5" : "translate-x-0.5"
              }`}
            />
          </button>
        )}
      </div>
      <div className="px-4 py-3 text-[12.5px] text-white/65">
        {settingKey ? (
          <p>
            Setting key:{" "}
            <code className="rounded bg-white/8 px-1.5 py-px font-mono text-[11.5px] text-white/85">
              {settingKey}
            </code>{" "}
            — toggling here is equivalent to flipping it on{" "}
            <code className="rounded bg-white/8 px-1.5 py-px font-mono text-[11.5px] text-white/85">
              /admin/settings
            </code>
            .
          </p>
        ) : (
          <p>
            Automatic kinds run on every new file/item; the only switch is
            removing the underlying dependency (e.g. clearing the TMDB key
            for {detail.display_name}).
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
  // The "trigger" case (no schedule row at all) is informational —
  // these are Automatic kinds that fire only on the on-add event,
  // never via the cron sweep. Keep the existing read-only copy.
  if (!detail.schedule) {
    return (
      <div className="rounded-lg border border-white/10 bg-white/2">
        <div className="border-b border-white/8 px-4 py-3">
          <div className="text-[13.5px] font-semibold text-white/95">
            Trigger
          </div>
          <div className="text-xs text-white/55">On-add only — no sweep cron.</div>
        </div>
        <div className="px-4 py-3 text-[12.5px] text-white/65">
          This kind runs only when the scanner emits a relevant event.
          There&apos;s no scheduled safety-net to configure.
        </div>
      </div>
    );
  }

  // Mount the editor with a key derived from the schedule signature
  // we want to treat as "baseline". When the operator clicks Save
  // (or navigates to a different kind), the key changes and the
  // editor remounts with the new initial values — no useEffect
  // syncing required, and in-progress edits aren't clobbered by
  // the 5s background polls.
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
    [
      frequency,
      enabled,
      windowSnap,
      initialFrequency,
      initialEnabled,
      initialWindow,
    ],
  );

  // `custom` is shown as a disabled <option> when the existing row
  // is a custom-cron — switching AWAY is fine, switching TO custom
  // would need a raw cron field we don't surface here.
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
    <div className="rounded-lg border border-white/10 bg-white/2">
      <div className="border-b border-white/8 px-4 py-3">
        <div className="text-[13.5px] font-semibold text-white/95">
          Schedule
        </div>
        <div className="text-xs text-white/55">
          When the safety-net sweep fires. On-add events ignore this — they
          fire as files appear.
        </div>
      </div>
      <div className="space-y-3 px-4 py-3 text-[12.5px]">
        <ScheduleField label="Frequency">
          <select
            value={frequency}
            onChange={(e) => setFrequency(e.target.value as TaskFrequency)}
            disabled={saving}
            className="w-full rounded border border-white/15 bg-black/30 px-2 py-1.5 text-[12.5px] text-white/90 focus:border-white/30 focus:outline-none"
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
        </ScheduleField>
        <ScheduleField label="Sweep enabled">
          <Toggle
            checked={enabled}
            disabled={saving}
            onChange={setEnabled}
            ariaLabel="Sweep enabled"
          />
        </ScheduleField>
        <ScheduleField
          label="Snap to maintenance window"
          hint="Defers heavy sweeps into the configured low-traffic window so they don't compete with playback."
        >
          <Toggle
            checked={windowSnap}
            disabled={saving}
            onChange={setWindowSnap}
            ariaLabel="Snap to maintenance window"
          />
        </ScheduleField>

        <div className="pt-1 text-[11.5px] text-white/55">
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

        <div className="flex items-center justify-end gap-2 border-t border-white/8 pt-3">
          <button
            type="button"
            onClick={reset}
            disabled={!dirty || saving}
            className="rounded border border-white/15 px-3 py-1.5 text-xs text-white/70 transition-colors hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            Reset
          </button>
          <button
            type="button"
            onClick={save}
            disabled={!dirty || saving}
            className="rounded bg-white/85 px-3 py-1.5 text-xs font-medium text-black transition-colors hover:bg-white disabled:cursor-not-allowed disabled:bg-white/20 disabled:text-white/55"
          >
            {saving ? "Saving…" : "Save schedule"}
          </button>
        </div>
      </div>
    </div>
  );
}

function ScheduleField({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid grid-cols-[160px_1fr] items-start gap-3">
      <div>
        <div className="text-[12px] font-medium text-white/85">{label}</div>
        {hint && (
          <div className="mt-0.5 text-[11px] text-white/45">{hint}</div>
        )}
      </div>
      <div className="flex items-center">{children}</div>
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
      className={`relative inline-flex h-5 w-10 cursor-pointer items-center rounded-full border transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
        checked
          ? "border-emerald-500/70 bg-emerald-500"
          : "border-white/20 bg-white/12"
      }`}
    >
      <span
        className={`block h-4 w-4 rounded-full bg-white shadow-sm transition-transform ${
          checked ? "translate-x-5" : "translate-x-0.5"
        }`}
      />
    </button>
  );
}


// ─── 30-day history chart ──────────────────────────────────────────────

function HistoryCard({ history }: { history: KindDetailDailyMetrics[] }) {
  return (
    <div className="rounded-lg border border-white/10 bg-white/2">
      <div className="flex items-baseline justify-between border-b border-white/8 px-4 py-3">
        <div>
          <div className="text-[13.5px] font-semibold text-white/95">
            History — 30 days
          </div>
          <div className="text-xs text-white/55">
            Targets processed per day · failures overlaid in red.
          </div>
        </div>
      </div>
      <div className="px-4 py-3">
        {history.length === 0 ? (
          <div className="rounded border border-dashed border-white/10 bg-white/3 px-4 py-8 text-center text-sm text-white/45">
            No rollup data yet. The daily rollup task runs at 02:00; the
            chart fills in as it completes its first runs.
          </div>
        ) : (
          <HistoryChart history={history} />
        )}
      </div>
    </div>
  );
}

function HistoryChart({ history }: { history: KindDetailDailyMetrics[] }) {
  // SVG bar chart, 600×100 viewBox. One bar per day for the
  // success count; small red dot at the top of any day with > 0
  // failures.
  const max = Math.max(
    1,
    ...history.map((d) => d.success_count + d.failure_count),
  );
  const barWidth = 580 / Math.max(history.length, 1);
  return (
    <svg viewBox="0 0 600 100" preserveAspectRatio="none" className="h-32 w-full">
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
              fill="#22c55e"
              opacity={0.9}
            />
            {d.failure_count > 0 && (
              <circle cx={x + barWidth / 2} cy={Math.max(y - 5, 6)} r={3.5} fill="#ef4444" />
            )}
          </g>
        );
      })}
    </svg>
  );
}

// ─── Recent runs ───────────────────────────────────────────────────────

function RecentRunsCard({ runs }: { runs: ActivityRecentRun[] }) {
  // Rows render ISO timestamps (deterministic; no nowMs needed).
  // Switch to relative deltas later by threading nowMs through.
  if (runs.length === 0) {
    return null;
  }
  return (
    <div className="overflow-hidden rounded-lg border border-white/10 bg-white/2">
      <div className="border-b border-white/8 px-4 py-3">
        <div className="text-[13.5px] font-semibold text-white/95">
          Recent runs
        </div>
        <div className="text-xs text-white/55">
          In-memory ring buffer · resets on restart.
        </div>
      </div>
      <div className="grid grid-cols-[12px_1fr_90px_100px_1fr] border-b border-white/8 bg-white/3 px-4 py-2 text-[10.5px] font-semibold uppercase tracking-[0.07em] text-white/55">
        <span />
        <span>Finished</span>
        <span>Duration</span>
        <span>Status</span>
        <span>Notes</span>
      </div>
      {runs.map((r, i) => (
        <div
          key={i}
          className="grid grid-cols-[12px_1fr_90px_100px_1fr] items-center gap-3 border-b border-white/8 px-4 py-2 text-[12.5px] last:border-b-0"
        >
          <span
            aria-hidden
            className={`block h-2 w-2 rounded-full ${r.success ? "bg-emerald-400" : "bg-red-400"}`}
          />
          <span className="text-white/80">
            {toIsoDateTime(r.finished_at_ms)}
          </span>
          <span className="font-mono tabular-nums text-white/65">
            {formatDurationMs(r.duration_ms)}
          </span>
          <span className="text-white/65">
            {r.success ? "ok" : prettyErrorClass(r.error_class)}
          </span>
          <span className="truncate text-[11.5px] text-white/45">
            {r.success ? "—" : (r.error_class ?? "unclassified")}
          </span>
        </div>
      ))}
    </div>
  );
}

// ─── Helpers (shared with activity client) ──────────────────────────────

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

