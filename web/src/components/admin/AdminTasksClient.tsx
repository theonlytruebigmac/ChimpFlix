"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import {
  admin as adminApi,
  type ScheduledTask,
  type ServerSettings,
  type ServerSettingsUpdate,
  type TaskFrequency,
  type TaskKindInfo,
  type TaskRun,
  type TasksListResponse,
} from "@/lib/chimpflix-api";

interface Props {
  initial: TasksListResponse;
  settings: ServerSettings;
}

const FREQUENCY_LABELS: Record<TaskFrequency, string> = {
  manual: "Manually (Run Now only)",
  hourly: "Every hour",
  every_3_hours: "Every 3 hours",
  every_6_hours: "Every 6 hours",
  every_12_hours: "Every 12 hours",
  daily: "Daily",
  every_3_days: "Every 3 days",
  weekly: "Weekly",
  monthly: "Monthly",
  on_change: "When media changes",
  custom: "Custom (cron)",
};

const FREQUENCY_OPTIONS: TaskFrequency[] = [
  "hourly",
  "every_3_hours",
  "every_6_hours",
  "every_12_hours",
  "daily",
  "every_3_days",
  "weekly",
  "monthly",
  "manual",
  "on_change",
  "custom",
];

/// Frequency labels rendered as a suffix in the simple-view rows
/// ("Backup database — daily"). Shorter than the dropdown labels
/// because the dropdown is hidden in simple mode.
const FREQUENCY_SHORT_LABELS: Record<TaskFrequency, string> = {
  manual: "manual",
  hourly: "hourly",
  every_3_hours: "every 3h",
  every_6_hours: "every 6h",
  every_12_hours: "every 12h",
  daily: "daily",
  every_3_days: "every 3 days",
  weekly: "weekly",
  monthly: "monthly",
  on_change: "on change",
  custom: "custom cron",
};

/// Kinds that always need a `library_id` parameter and therefore
/// don't have a meaningful "global" instance to show in the simple
/// toggle list. Per-library scans are triggered from the library
/// admin page instead; the simple view shows a footer link there.
const PER_LIBRARY_KINDS = new Set(["scan_library"]);

/// Find the task row that represents the "global" instance of a
/// kind in the simple toggle view. A row counts as global when it
/// either has no `library_id` in its params or has the literal
/// empty-object params (`{}`). Custom per-library task rows still
/// show in the advanced view but don't drive the simple toggle.
function findGlobalTaskFor(kind: string, tasks: ScheduledTask[]): ScheduledTask | null {
  return (
    tasks.find((t) => {
      if (t.kind !== kind) return false;
      try {
        const parsed = JSON.parse(t.params_json || "{}");
        return parsed && typeof parsed === "object" && parsed.library_id == null;
      } catch {
        // Garbage params — surface in advanced; never count as global.
        return false;
      }
    }) ?? null
  );
}

export function AdminTasksClient({ initial, settings }: Props) {
  const [tasks, setTasks] = useState(initial.tasks);
  const [kinds] = useState(initial.kinds);
  const [windowStart, setWindowStart] = useState(settings.maintenance_window_start);
  const [windowEnd, setWindowEnd] = useState(settings.maintenance_window_end);
  const [savedWindowStart, setSavedWindowStart] = useState(settings.maintenance_window_start);
  const [savedWindowEnd, setSavedWindowEnd] = useState(settings.maintenance_window_end);
  const [savingWindow, setSavingWindow] = useState(false);
  const [showAdd, setShowAdd] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Plex-style "Hide Advanced" button. Simple mode is a flat
  // checkbox list of housekeeping tasks with baked-in frequencies;
  // Advanced mode is the per-card editor that lets an operator
  // tweak frequency, custom cron, params, etc. Default to simple
  // because that's the path the user lands on after reading the
  // admin docs — Advanced is power-user territory.
  const [advanced, setAdvanced] = useState(false);

  const windowDirty = windowStart !== savedWindowStart || windowEnd !== savedWindowEnd;
  const windowActiveNow = isWithinWindow(savedWindowStart, savedWindowEnd, new Date());

  async function refresh() {
    try {
      const next = await adminApi.tasks.list();
      setTasks(next.tasks);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function saveWindow() {
    setSavingWindow(true);
    setError(null);
    try {
      const patch: ServerSettingsUpdate = {
        maintenance_window_start: windowStart,
        maintenance_window_end: windowEnd,
      };
      await adminApi.settings.patch(patch);
      setSavedWindowStart(windowStart);
      setSavedWindowEnd(windowEnd);
      // Window change might shift `next_run_at` for any
      // window-eligible task — pull fresh so the cards show the
      // new schedule.
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSavingWindow(false);
    }
  }

  return (
    <div className="space-y-6">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      <section className="rounded-lg border border-white/10 bg-white/2 p-4 sm:p-5">
        <div className="flex flex-wrap items-baseline justify-between gap-2">
          <div>
            <h2 className="text-base font-semibold">Maintenance window</h2>
            <p className="mt-1 max-w-xl text-xs text-white/55">
              Heavy background tasks (full scans, metadata refresh, backups)
              run inside this window so they don&apos;t compete with playback.
              Server-local time. If the end time is earlier than the start
              the window wraps midnight.
            </p>
          </div>
          <span
            className={`rounded px-2 py-0.5 text-[11px] uppercase tracking-wider ${
              windowActiveNow
                ? "bg-emerald-500/15 text-emerald-300"
                : "bg-white/10 text-white/60"
            }`}
          >
            {windowActiveNow ? "Active now" : `Next: ${nextWindowDescription(savedWindowStart, savedWindowEnd)}`}
          </span>
        </div>
        <div className="mt-4 flex flex-wrap items-center gap-3">
          <div className="flex items-center gap-2 text-sm">
            <span className="text-white/55">From</span>
            <input
              type="time"
              value={windowStart}
              onChange={(e) => setWindowStart(e.target.value)}
              className="rounded border border-white/10 bg-black/30 px-2 py-1 text-sm tabular-nums"
            />
            <span className="text-white/55">to</span>
            <input
              type="time"
              value={windowEnd}
              onChange={(e) => setWindowEnd(e.target.value)}
              className="rounded border border-white/10 bg-black/30 px-2 py-1 text-sm tabular-nums"
            />
          </div>
          <button
            disabled={!windowDirty || savingWindow}
            onClick={saveWindow}
            className="rounded-md bg-red-500 px-3 py-1.5 text-sm font-semibold text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
          >
            {savingWindow ? "Saving…" : "Save window"}
          </button>
        </div>
      </section>

      <div className="flex items-center justify-between">
        <span className="text-sm text-white/60">
          {advanced
            ? `${tasks.length} task${tasks.length === 1 ? "" : "s"} configured`
            : "Toggle a task to enable it. Hidden defaults match what Plex ships."}
        </span>
        <button
          onClick={() => setAdvanced((v) => !v)}
          className="rounded border border-white/15 px-3 py-1.5 text-xs text-white/80 hover:bg-white/5"
        >
          {advanced ? "Hide Advanced" : "Show Advanced"}
        </button>
      </div>

      {!advanced && (
        <SimpleTaskList
          kinds={kinds}
          tasks={tasks}
          onChanged={refresh}
          onError={setError}
        />
      )}

      {advanced && (
        <>
          <div className="flex items-center justify-end">
            <button
              onClick={() => setShowAdd((v) => !v)}
              className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:px-3 sm:py-1.5 text-white hover:bg-red-600"
            >
              {showAdd ? "Cancel" : "+ New task"}
            </button>
          </div>

          {showAdd && (
            <NewTaskForm
              kinds={kinds}
              onCreated={async () => {
                setShowAdd(false);
                await refresh();
              }}
              onError={setError}
            />
          )}

          <div className="space-y-3">
            {tasks.length === 0 && !showAdd && (
              <div className="rounded-lg border border-dashed border-white/15 bg-white/2 p-8 text-center text-sm text-white/50">
                No scheduled tasks. Click &quot;+ New task&quot; to add one.
              </div>
            )}
            {tasks.map((t) => (
              <TaskRow
                key={t.id}
                task={t}
                kinds={kinds}
                onChanged={refresh}
                onError={setError}
              />
            ))}
          </div>
        </>
      )}
    </div>
  );
}

/// Plex-style flat-list view: one row per registered task kind with a
/// checkbox + inline status + Run now button. Toggling on creates the
/// global instance of the task using the registry's recommended
/// frequency; toggling off disables it (preserves customizations so a
/// later re-enable doesn't lose the operator's Advanced edits).
function SimpleTaskList({
  kinds,
  tasks,
  onChanged,
  onError,
}: {
  kinds: TaskKindInfo[];
  tasks: ScheduledTask[];
  onChanged: () => Promise<void>;
  onError: (msg: string | null) => void;
}) {
  // Filter to the kinds that have a meaningful global instance.
  // Per-library tasks (scan, currently only scan_library) are
  // surfaced from the library admin page instead.
  const visibleKinds = useMemo(
    () => kinds.filter((k) => !PER_LIBRARY_KINDS.has(k.kind)),
    [kinds],
  );

  return (
    <section className="rounded-lg border border-white/10 bg-white/2">
      <ul className="divide-y divide-white/5">
        {visibleKinds.map((k) => (
          <SimpleTaskRow
            key={k.kind}
            kind={k}
            existing={findGlobalTaskFor(k.kind, tasks)}
            onChanged={onChanged}
            onError={onError}
          />
        ))}
      </ul>
      <div className="border-t border-white/10 px-4 py-3 text-xs text-white/45">
        Per-library scans run automatically via the file watcher. Trigger
        them by hand from the relevant library card in{" "}
        <code className="font-mono text-white/55">Library &rsaquo; Libraries</code>.
      </div>
    </section>
  );
}

function SimpleTaskRow({
  kind,
  existing,
  onChanged,
  onError,
}: {
  kind: TaskKindInfo;
  existing: ScheduledTask | null;
  onChanged: () => Promise<void>;
  onError: (msg: string | null) => void;
}) {
  const [busy, setBusy] = useState(false);
  // Handle for the post-runNow refresh delay. Tracked so an unmount
  // mid-wait cancels the pending setState.
  const runNowTimerRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      if (runNowTimerRef.current !== null) {
        window.clearTimeout(runNowTimerRef.current);
        runNowTimerRef.current = null;
      }
    };
  }, []);

  const enabled = existing?.enabled ?? false;
  const frequency = (existing?.frequency ?? kind.default_frequency) as TaskFrequency;
  const requiresWindow =
    existing?.requires_maintenance_window ?? kind.default_requires_maintenance_window;
  const freqLabel = FREQUENCY_SHORT_LABELS[frequency] ?? frequency;
  const scheduleSummary = requiresWindow
    ? `${freqLabel} · in maintenance window`
    : freqLabel;

  async function toggle(next: boolean) {
    setBusy(true);
    onError(null);
    try {
      if (existing) {
        // Row already in place — flip the enabled flag. We never
        // delete on disable so an operator's Advanced customisations
        // (custom cron, non-default params, frequency tweaks) ride
        // through a later re-enable unchanged.
        await adminApi.tasks.update(existing.id, { enabled: next });
      } else if (next) {
        // First time the operator turns this on — create the row
        // with the registry's recommended defaults.
        await adminApi.tasks.create({
          kind: kind.kind,
          name: kind.display_name,
          frequency: kind.default_frequency,
          requires_maintenance_window: kind.default_requires_maintenance_window,
          params_json: "{}",
          enabled: true,
        });
      }
      await onChanged();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function runNow() {
    if (!existing) {
      // Toggle on first, then trigger — saves the operator one click.
      await toggle(true);
      // Find the freshly-created row by re-querying. Cheap: the
      // toggle already refetched.
      return;
    }
    setBusy(true);
    onError(null);
    try {
      await adminApi.tasks.runNow(existing.id);
      // Give the runner a beat to start writing history, then refresh.
      if (runNowTimerRef.current !== null) {
        window.clearTimeout(runNowTimerRef.current);
      }
      runNowTimerRef.current = window.setTimeout(() => {
        runNowTimerRef.current = null;
        void onChanged();
        setBusy(false);
      }, 700);
      return;
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  }

  return (
    <li className="flex flex-wrap items-center gap-3 px-4 py-3 sm:flex-nowrap">
      <label className="flex flex-1 min-w-0 items-start gap-3">
        <input
          type="checkbox"
          checked={enabled}
          disabled={busy}
          onChange={(e) => toggle(e.target.checked)}
          className="mt-1"
        />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-baseline gap-2 text-sm">
            <span className="font-medium">{kind.display_name}</span>
            <span className="text-xs text-white/45">— {scheduleSummary}</span>
            {existing?.last_status === "failed" && (
              <span className="rounded bg-red-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-red-300">
                Last run failed
              </span>
            )}
            {existing?.last_status === "running" && (
              <span className="rounded bg-blue-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-blue-300">
                Running
              </span>
            )}
          </div>
          <p className="mt-0.5 text-xs text-white/55">{kind.description}</p>
          <div className="mt-1 text-xs text-white/40">
            {existing && enabled && (
              <>
                Next: {formatWhen(existing.next_run_at)}
                {existing.last_run_at && (
                  <> · Last: {formatWhen(existing.last_run_at)}</>
                )}
              </>
            )}
            {existing && !enabled && <>Disabled — toggle to resume scheduling.</>}
            {!existing && <>Not yet enabled.</>}
          </div>
          {existing?.last_error && (
            <div className="mt-0.5 truncate text-xs text-red-300" title={existing.last_error}>
              {existing.last_error}
            </div>
          )}
        </div>
      </label>
      {existing && (
        <button
          disabled={busy}
          onClick={runNow}
          className="rounded border border-white/15 px-2 py-1 text-xs text-white/80 hover:bg-white/5 disabled:opacity-50"
        >
          Run now
        </button>
      )}
    </li>
  );
}

function TaskRow({
  task,
  kinds,
  onChanged,
  onError,
}: {
  task: ScheduledTask;
  kinds: TaskKindInfo[];
  onChanged: () => Promise<void>;
  onError: (msg: string | null) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [name, setName] = useState(task.name);
  const [frequency, setFrequency] = useState<TaskFrequency>(task.frequency);
  const [requiresWindow, setRequiresWindow] = useState(task.requires_maintenance_window);
  const [cron, setCron] = useState(task.cron_expr);
  const [params, setParams] = useState(task.params_json);
  const [busy, setBusy] = useState(false);
  const [runs, setRuns] = useState<TaskRun[] | null>(null);
  // Tracked so unmount mid-wait cancels the pending refresh.
  const taskCardRunNowTimerRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      if (taskCardRunNowTimerRef.current !== null) {
        window.clearTimeout(taskCardRunNowTimerRef.current);
        taskCardRunNowTimerRef.current = null;
      }
    };
  }, []);

  const kindInfo = kinds.find((k) => k.kind === task.kind);
  const dirty =
    name !== task.name ||
    frequency !== task.frequency ||
    requiresWindow !== task.requires_maintenance_window ||
    (frequency === "custom" && cron !== task.cron_expr) ||
    params !== task.params_json;

  async function save() {
    setBusy(true);
    onError(null);
    try {
      const patch: Parameters<typeof adminApi.tasks.update>[1] = {
        name,
        frequency,
        requires_maintenance_window: requiresWindow,
        params_json: params,
      };
      // Only send cron when the user is in custom mode — otherwise the
      // existing placeholder is preserved unchanged.
      if (frequency === "custom") patch.cron_expr = cron;
      await adminApi.tasks.update(task.id, patch);
      await onChanged();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function toggleEnabled() {
    setBusy(true);
    onError(null);
    try {
      await adminApi.tasks.update(task.id, { enabled: !task.enabled });
      await onChanged();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function runNow() {
    setBusy(true);
    onError(null);
    try {
      await adminApi.tasks.runNow(task.id);
      // Give the runner a beat to start and write history.
      if (taskCardRunNowTimerRef.current !== null) {
        window.clearTimeout(taskCardRunNowTimerRef.current);
      }
      taskCardRunNowTimerRef.current = window.setTimeout(async () => {
        taskCardRunNowTimerRef.current = null;
        await onChanged();
        if (expanded) {
          const r = await adminApi.tasks.listRuns(task.id, 20);
          setRuns(r.runs);
        }
        setBusy(false);
      }, 700);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  }

  async function remove() {
    if (!window.confirm(`Delete task "${task.name}"?`)) return;
    setBusy(true);
    onError(null);
    try {
      await adminApi.tasks.delete(task.id);
      await onChanged();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function loadRuns() {
    if (runs !== null) return;
    try {
      const r = await adminApi.tasks.listRuns(task.id, 20);
      setRuns(r.runs);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    }
  }

  const scheduleSummary = useMemo(() => {
    if (task.frequency === "custom") {
      return `Custom · ${task.cron_expr}`;
    }
    const base = FREQUENCY_LABELS[task.frequency];
    return task.requires_maintenance_window
      ? `${base} · in maintenance window`
      : base;
  }, [task.frequency, task.cron_expr, task.requires_maintenance_window]);

  const neverFires =
    task.frequency === "manual" || task.frequency === "on_change";

  return (
    <div className="rounded-lg border border-white/10 bg-white/2">
      <div className="flex flex-wrap items-center gap-3 p-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2 text-sm">
            <span className="font-medium">{task.name}</span>
            <span className="rounded bg-white/10 px-1.5 py-0.5 font-mono text-[10px] text-white/60">
              {task.kind}
            </span>
            <StatusBadge status={task.last_status} />
            {!task.enabled && (
              <span className="rounded bg-white/10 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-white/50">
                Off
              </span>
            )}
          </div>
          <div className="mt-0.5 text-xs text-white/55">{scheduleSummary}</div>
          <div className="mt-0.5 text-xs text-white/40">
            {neverFires ? (
              <>Runs only when triggered.</>
            ) : (
              <>Next: {formatWhen(task.next_run_at)}</>
            )}
            {task.last_run_at && (
              <>
                {" · "}Last:{" "}
                {formatWhen(task.last_run_at)}
                {task.last_duration_ms != null && (
                  <> ({formatDuration(task.last_duration_ms)})</>
                )}
              </>
            )}
          </div>
          {task.last_error && (
            <div className="mt-0.5 truncate text-xs text-red-300" title={task.last_error}>
              {task.last_error}
            </div>
          )}
        </div>
        <button
          disabled={busy}
          onClick={runNow}
          className="rounded border border-white/15 px-2 py-1 text-xs text-white/80 hover:bg-white/5"
        >
          Run now
        </button>
        <button
          disabled={busy}
          onClick={toggleEnabled}
          className={`rounded border px-2 py-1 text-xs ${task.enabled ? "border-emerald-500/40 text-emerald-300" : "border-white/15 text-white/50"}`}
        >
          {task.enabled ? "Enabled" : "Disabled"}
        </button>
        <button
          onClick={() => {
            setExpanded((v) => !v);
            if (!expanded) loadRuns();
          }}
          className="rounded border border-white/15 px-2 py-1 text-xs text-white/80 hover:bg-white/5"
        >
          {expanded ? "Collapse" : "Edit ▾"}
        </button>
      </div>
      {expanded && (
        <div className="space-y-4 border-t border-white/10 p-4">
          {kindInfo?.description && (
            <p className="text-xs text-white/55">{kindInfo.description}</p>
          )}
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
            <Field label="Name">
              <input
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
              />
            </Field>
            <Field label="Run">
              <FrequencyPicker value={frequency} onChange={setFrequency} />
            </Field>
            {frequency === "custom" && (
              <Field
                label="Custom cron"
                hint="Six fields: sec min hour dom mon dow (UTC). Note: the maintenance-window times above are server-LOCAL; if your server's timezone isn't UTC, a cron time and a window time that look the same on paper won't actually overlap. Pick a frequency above unless you need cron's flexibility."
              >
                <input
                  type="text"
                  value={cron}
                  onChange={(e) => setCron(e.target.value)}
                  className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
                  placeholder="0 0 3 * * *"
                />
              </Field>
            )}
            <Field
              label="Maintenance window"
              hint="When on, the next scheduled run is deferred to the next opening of the server maintenance window."
            >
              <label className="flex items-center gap-2 rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm">
                <input
                  type="checkbox"
                  checked={requiresWindow}
                  disabled={frequency === "manual" || frequency === "on_change"}
                  onChange={(e) => setRequiresWindow(e.target.checked)}
                />
                <span>Only run inside the maintenance window</span>
              </label>
            </Field>
            <Field label="Params (JSON)" hint={kindInfo?.params_schema}>
              <input
                type="text"
                value={params}
                onChange={(e) => setParams(e.target.value)}
                className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
              />
            </Field>
          </div>
          <div className="flex items-center gap-3">
            <button
              disabled={!dirty || busy}
              onClick={save}
              className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:px-3 sm:py-1.5 text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
            >
              Save
            </button>
            <button
              disabled={busy}
              onClick={remove}
              className="rounded-md border border-red-500/40 px-3 py-1.5 text-sm text-red-300 hover:bg-red-500/10 disabled:opacity-50"
            >
              Delete
            </button>
          </div>

          <div>
            <div className="mb-2 text-xs uppercase tracking-wider text-white/40">
              Run history
            </div>
            {runs == null ? (
              <div className="text-sm text-white/40">Loading…</div>
            ) : runs.length === 0 ? (
              <div className="text-sm text-white/40">No runs recorded yet.</div>
            ) : (
              <div className="overflow-hidden rounded border border-white/10">
                <table className="w-full text-xs">
                  <thead className="bg-white/5 text-left text-white/40">
                    <tr>
                      <th className="px-3 py-1.5">Started</th>
                      <th className="px-3 py-1.5">Status</th>
                      <th className="px-3 py-1.5">Duration</th>
                      <th className="px-3 py-1.5">Output</th>
                    </tr>
                  </thead>
                  <tbody>
                    {runs.map((r) => (
                      <tr key={r.id} className="border-t border-white/5">
                        <td className="px-3 py-1.5 text-white/60">
                          {formatWhen(r.started_at)}
                        </td>
                        <td className="px-3 py-1.5">
                          <StatusBadge status={r.status} />
                        </td>
                        <td className="px-3 py-1.5 tabular-nums text-white/60">
                          {r.finished_at != null
                            ? formatDuration(r.finished_at - r.started_at)
                            : "—"}
                        </td>
                        <td className="px-3 py-1.5 text-white/70">
                          {r.error ? (
                            <span className="text-red-300">{r.error}</span>
                          ) : r.log ? (
                            <code className="font-mono text-[11px]">
                              {r.log}
                            </code>
                          ) : (
                            "—"
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function NewTaskForm({
  kinds,
  onCreated,
  onError,
}: {
  kinds: TaskKindInfo[];
  onCreated: () => Promise<void>;
  onError: (msg: string | null) => void;
}) {
  const [kind, setKind] = useState(kinds[0]?.kind ?? "prune_sessions");
  const [name, setName] = useState("");
  const kindInfo = kinds.find((k) => k.kind === kind);
  // Each kind has a recommended default schedule/window combo from
  // the registry — we mirror them here so the form is pre-filled
  // sensibly when the operator switches kind.
  const [frequency, setFrequency] = useState<TaskFrequency>(
    kindInfo?.default_frequency ?? "daily",
  );
  const [requiresWindow, setRequiresWindow] = useState(
    kindInfo?.default_requires_maintenance_window ?? false,
  );
  const [cron, setCron] = useState("0 0 3 * * *");
  const [params, setParams] = useState("{}");
  const [busy, setBusy] = useState(false);

  function changeKind(nextKind: string) {
    setKind(nextKind);
    const info = kinds.find((k) => k.kind === nextKind);
    if (info) {
      setFrequency(info.default_frequency);
      setRequiresWindow(info.default_requires_maintenance_window);
    }
  }

  async function submit() {
    setBusy(true);
    onError(null);
    try {
      await adminApi.tasks.create({
        kind,
        name: name.trim() || (kindInfo?.display_name ?? kind),
        frequency,
        requires_maintenance_window: requiresWindow,
        cron_expr: frequency === "custom" ? cron : undefined,
        params_json: params,
        enabled: true,
      });
      await onCreated();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-4 rounded-lg border border-white/10 bg-white/2 p-4">
      <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
        <Field label="Kind" hint={kindInfo?.description}>
          <select
            value={kind}
            onChange={(e) => changeKind(e.target.value)}
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
          >
            {kinds.map((k) => (
              <option key={k.kind} value={k.kind}>
                {k.display_name}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Name">
          <input
            type="text"
            value={name}
            placeholder={kindInfo?.display_name}
            onChange={(e) => setName(e.target.value)}
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
          />
        </Field>
        <Field label="Run">
          <FrequencyPicker value={frequency} onChange={setFrequency} />
        </Field>
        <Field label="Maintenance window">
          <label className="flex items-center gap-2 rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm">
            <input
              type="checkbox"
              checked={requiresWindow}
              disabled={frequency === "manual" || frequency === "on_change"}
              onChange={(e) => setRequiresWindow(e.target.checked)}
            />
            <span>Only run inside the maintenance window</span>
          </label>
        </Field>
        {frequency === "custom" && (
          <Field
            label="Custom cron"
            hint="Six fields: sec min hour dom mon dow (UTC). The maintenance-window times above are server-LOCAL — on a non-UTC server, a cron time and a window time that look the same on paper won't actually overlap."
          >
            <input
              type="text"
              value={cron}
              onChange={(e) => setCron(e.target.value)}
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
            />
          </Field>
        )}
        <Field label="Params (JSON)" hint={kindInfo?.params_schema}>
          <input
            type="text"
            value={params}
            onChange={(e) => setParams(e.target.value)}
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
          />
        </Field>
      </div>
      <button
        disabled={busy}
        onClick={submit}
        className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:px-3 sm:py-1.5 text-white hover:bg-red-600 disabled:opacity-50"
      >
        {busy ? "Creating…" : "Create"}
      </button>
    </div>
  );
}

function FrequencyPicker({
  value,
  onChange,
}: {
  value: TaskFrequency;
  onChange: (next: TaskFrequency) => void;
}) {
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value as TaskFrequency)}
      className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
    >
      {FREQUENCY_OPTIONS.map((f) => (
        <option key={f} value={f}>
          {FREQUENCY_LABELS[f]}
        </option>
      ))}
    </select>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <label className="mb-1 block text-xs font-medium uppercase tracking-wider text-white/50">
        {label}
      </label>
      {children}
      {hint && <p className="mt-1 text-xs text-white/50">{hint}</p>}
    </div>
  );
}

function StatusBadge({ status }: { status: string | null }) {
  if (status == null) {
    return (
      <span className="rounded bg-white/10 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-white/50">
        New
      </span>
    );
  }
  const cls =
    status === "success"
      ? "bg-emerald-500/15 text-emerald-300"
      : status === "running"
        ? "bg-blue-500/15 text-blue-300"
        : status === "failed"
          ? "bg-red-500/15 text-red-300"
          : "bg-white/10 text-white/60";
  return (
    <span
      className={`rounded px-1.5 py-0.5 text-[10px] uppercase tracking-wider ${cls}`}
    >
      {status}
    </span>
  );
}

function formatWhen(epochMs: number): string {
  // Anything more than 50 years out (the scheduler's "never" sentinel
  // is year 2100) renders as a friendly placeholder — operators
  // shouldn't ever see "1/1/2100, 12:00:00 AM" in the UI.
  if (epochMs > Date.now() + 50 * 365 * 86_400_000) return "never";
  return new Date(epochMs).toLocaleString();
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const mins = Math.floor(ms / 60_000);
  const secs = Math.floor((ms % 60_000) / 1000);
  return `${mins}m ${secs}s`;
}

/// Decide whether the local clock currently falls inside the maintenance
/// window. Mirrors the server-side `snap_to_maintenance_window` logic
/// (including midnight-wrap handling) so the "Active now" badge stays
/// honest. We compute minutes-since-midnight to dodge timezone math.
function isWithinWindow(start: string, end: string, now: Date): boolean {
  const s = hhmmToMinutes(start);
  const e = hhmmToMinutes(end);
  if (s == null || e == null) return false;
  const nowMin = now.getHours() * 60 + now.getMinutes();
  if (s === e) return false;
  if (s < e) return nowMin >= s && nowMin < e;
  // Wraps midnight.
  return nowMin >= s || nowMin < e;
}

function nextWindowDescription(start: string, end: string): string {
  const s = hhmmToMinutes(start);
  const e = hhmmToMinutes(end);
  if (s == null || e == null) return start;
  return `${start} → ${end}`;
}

function hhmmToMinutes(hhmm: string): number | null {
  const parts = hhmm.split(":");
  if (parts.length !== 2) return null;
  const h = parseInt(parts[0], 10);
  const m = parseInt(parts[1], 10);
  if (Number.isNaN(h) || Number.isNaN(m)) return null;
  if (h < 0 || h >= 24 || m < 0 || m >= 60) return null;
  return h * 60 + m;
}
