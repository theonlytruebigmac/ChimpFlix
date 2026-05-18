"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type ScheduledTask,
  type TaskKindInfo,
  type TaskRun,
  type TasksListResponse,
} from "@/lib/chimpflix-api";

export function AdminTasksClient({ initial }: { initial: TasksListResponse }) {
  const [tasks, setTasks] = useState(initial.tasks);
  const [kinds] = useState(initial.kinds);
  const [showAdd, setShowAdd] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function refresh() {
    try {
      const next = await adminApi.tasks.list();
      setTasks(next.tasks);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div className="space-y-6">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      <div className="flex items-center justify-between">
        <span className="text-sm text-white/60">
          {tasks.length} task{tasks.length === 1 ? "" : "s"}
        </span>
        <button
          onClick={() => setShowAdd((v) => !v)}
          className="rounded-md bg-red-500 px-3 py-1.5 text-sm font-semibold text-white hover:bg-red-600"
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
    </div>
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
  const [cron, setCron] = useState(task.cron_expr);
  const [params, setParams] = useState(task.params_json);
  const [busy, setBusy] = useState(false);
  const [runs, setRuns] = useState<TaskRun[] | null>(null);

  const kindInfo = kinds.find((k) => k.kind === task.kind);
  const dirty =
    name !== task.name || cron !== task.cron_expr || params !== task.params_json;

  async function save() {
    setBusy(true);
    onError(null);
    try {
      await adminApi.tasks.update(task.id, { name, cron_expr: cron, params_json: params });
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
      setTimeout(async () => {
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

  return (
    <div className="rounded-lg border border-white/10 bg-white/2">
      <div className="flex items-center gap-3 p-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-sm">
            <span className="font-medium">{task.name}</span>
            <span className="rounded bg-white/10 px-1.5 py-0.5 font-mono text-[10px] text-white/60">
              {task.kind}
            </span>
            <StatusBadge status={task.last_status} />
          </div>
          <div className="mt-0.5 font-mono text-xs text-white/40">{task.cron_expr}</div>
          <div className="mt-0.5 text-xs text-white/40">
            Next: {formatWhen(task.next_run_at)}
            {task.last_run_at && (
              <>
                {" · "}Last:{" "}
                {formatWhen(task.last_run_at)}
                {task.last_duration_ms != null && (
                  <> ({task.last_duration_ms}ms)</>
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
          <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
            <Field label="Name">
              <input
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
              />
            </Field>
            <Field label="Schedule" hint="Pick a preset or use Custom for raw cron.">
              <CronEditor value={cron} onChange={setCron} />
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
              className="rounded-md bg-red-500 px-3 py-1.5 text-sm font-semibold text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
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
                            ? `${r.finished_at - r.started_at}ms`
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
  const [cron, setCron] = useState("0 0 * * * *");
  const [params, setParams] = useState("{}");
  const [busy, setBusy] = useState(false);

  const kindInfo = kinds.find((k) => k.kind === kind);

  async function submit() {
    setBusy(true);
    onError(null);
    try {
      await adminApi.tasks.create({
        kind,
        name: name.trim() || (kindInfo?.display_name ?? kind),
        cron_expr: cron,
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
            onChange={(e) => setKind(e.target.value)}
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
        <Field label="Schedule">
          <CronEditor value={cron} onChange={setCron} />
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
      <button
        disabled={busy}
        onClick={submit}
        className="rounded-md bg-red-500 px-3 py-1.5 text-sm font-semibold text-white hover:bg-red-600 disabled:opacity-50"
      >
        {busy ? "Creating…" : "Create"}
      </button>
    </div>
  );
}

/// Structured editor for our 6-field cron strings
/// (`sec min hour dom mon dow`). Defaults to the most-common
/// preset shapes (Hourly / Every N hours / Daily at time / Weekly
/// on day at time / Custom raw expression). When the value can't
/// be parsed into a known shape, the mode flips to Custom and the
/// raw input is the source of truth.
///
/// Day-of-week numbering: 0 = Sunday … 6 = Saturday (cron standard).
function CronEditor({
  value,
  onChange,
}: {
  value: string;
  onChange: (next: string) => void;
}) {
  const parsed = parseCron(value);
  const mode = parsed.mode;

  function setMode(next: CronMode) {
    if (next === "hourly") {
      onChange("0 0 * * * *");
    } else if (next === "every_n_hours") {
      onChange("0 0 */6 * * *");
    } else if (next === "daily") {
      onChange("0 0 3 * * *");
    } else if (next === "weekly") {
      onChange("0 0 4 * * 0");
    } else if (next === "every_n_minutes") {
      onChange("0 */30 * * * *");
    } else {
      // Custom — keep the current cron as-is so the user can edit it.
    }
  }

  return (
    <div className="space-y-2">
      <select
        value={mode}
        onChange={(e) => setMode(e.target.value as CronMode)}
        className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
      >
        <option value="every_n_minutes">Every N minutes</option>
        <option value="hourly">Every hour</option>
        <option value="every_n_hours">Every N hours</option>
        <option value="daily">Daily at time</option>
        <option value="weekly">Weekly on day at time</option>
        <option value="custom">Custom cron expression</option>
      </select>

      {mode === "every_n_minutes" && (
        <div className="flex items-center gap-2 text-sm">
          <span className="text-white/55">Every</span>
          <NumberInput
            value={parsed.everyMinutes ?? 30}
            min={1}
            max={59}
            onChange={(n) => onChange(`0 */${n} * * * *`)}
          />
          <span className="text-white/55">minutes</span>
        </div>
      )}

      {mode === "every_n_hours" && (
        <div className="flex items-center gap-2 text-sm">
          <span className="text-white/55">Every</span>
          <NumberInput
            value={parsed.everyHours ?? 6}
            min={1}
            max={23}
            onChange={(n) => onChange(`0 0 */${n} * * *`)}
          />
          <span className="text-white/55">hours</span>
        </div>
      )}

      {mode === "daily" && (
        <div className="flex items-center gap-2 text-sm">
          <span className="text-white/55">At</span>
          <TimeInput
            hour={parsed.hour ?? 3}
            minute={parsed.minute ?? 0}
            onChange={(h, m) => onChange(`0 ${m} ${h} * * *`)}
          />
        </div>
      )}

      {mode === "weekly" && (
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <span className="text-white/55">On</span>
          <select
            value={parsed.dayOfWeek ?? 0}
            onChange={(e) =>
              onChange(
                `0 ${parsed.minute ?? 0} ${parsed.hour ?? 4} * * ${e.target.value}`,
              )
            }
            className="rounded border border-white/10 bg-black/30 px-2 py-1 text-sm"
          >
            {DAYS.map((d, i) => (
              <option key={i} value={i}>
                {d}
              </option>
            ))}
          </select>
          <span className="text-white/55">at</span>
          <TimeInput
            hour={parsed.hour ?? 4}
            minute={parsed.minute ?? 0}
            onChange={(h, m) =>
              onChange(`0 ${m} ${h} * * ${parsed.dayOfWeek ?? 0}`)
            }
          />
        </div>
      )}

      {mode === "custom" && (
        <input
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
          placeholder="sec min hour day-of-month month day-of-week"
        />
      )}

      <div className="font-mono text-[11px] text-white/40">
        {value}
        {parsed.summary && (
          <span className="ml-2 font-sans text-white/55">
            ({parsed.summary})
          </span>
        )}
      </div>
    </div>
  );
}

const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

type CronMode =
  | "every_n_minutes"
  | "hourly"
  | "every_n_hours"
  | "daily"
  | "weekly"
  | "custom";

interface ParsedCron {
  mode: CronMode;
  everyMinutes?: number;
  everyHours?: number;
  hour?: number;
  minute?: number;
  dayOfWeek?: number;
  summary?: string;
}

/// Detect which of the friendly modes a raw cron string fits into.
/// Returns `custom` for anything we don't recognise — the editor
/// then shows the raw input box and the user can write any
/// expression they like. The summary is a human-friendly description
/// shown alongside the raw value so the operator can sanity-check
/// what the cron evaluates to.
function parseCron(value: string): ParsedCron {
  const parts = value.trim().split(/\s+/);
  if (parts.length !== 6) return { mode: "custom" };
  const [sec, min, hour, dom, mon, dow] = parts;
  if (sec !== "0" || dom !== "*" || mon !== "*") {
    return { mode: "custom" };
  }

  // Every-N-minutes shape: `0 */N * * * *`
  const minEvery = min.match(/^\*\/(\d+)$/);
  if (minEvery && hour === "*" && dow === "*") {
    return {
      mode: "every_n_minutes",
      everyMinutes: parseInt(minEvery[1], 10),
      summary: `Every ${minEvery[1]} minutes`,
    };
  }

  // Hourly shape: `0 0 * * * *`
  if (min === "0" && hour === "*" && dow === "*") {
    return { mode: "hourly", summary: "Every hour" };
  }

  // Every-N-hours shape: `0 0 */N * * *`
  const hourEvery = hour.match(/^\*\/(\d+)$/);
  if (hourEvery && min === "0" && dow === "*") {
    return {
      mode: "every_n_hours",
      everyHours: parseInt(hourEvery[1], 10),
      summary: `Every ${hourEvery[1]} hours`,
    };
  }

  // Daily shape: `0 MM HH * * *`
  const m = parseInt(min, 10);
  const h = parseInt(hour, 10);
  if (
    !Number.isNaN(m) && !Number.isNaN(h) && dow === "*"
    && m >= 0 && m < 60 && h >= 0 && h < 24
  ) {
    return {
      mode: "daily",
      hour: h,
      minute: m,
      summary: `Daily at ${pad2(h)}:${pad2(m)}`,
    };
  }

  // Weekly shape: `0 MM HH * * D`
  const d = parseInt(dow, 10);
  if (
    !Number.isNaN(m) && !Number.isNaN(h) && !Number.isNaN(d)
    && m >= 0 && m < 60 && h >= 0 && h < 24 && d >= 0 && d < 7
  ) {
    return {
      mode: "weekly",
      hour: h,
      minute: m,
      dayOfWeek: d,
      summary: `Weekly on ${DAYS[d]} at ${pad2(h)}:${pad2(m)}`,
    };
  }

  return { mode: "custom" };
}

function pad2(n: number): string {
  return n.toString().padStart(2, "0");
}

function NumberInput({
  value,
  min,
  max,
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  onChange: (n: number) => void;
}) {
  return (
    <input
      type="number"
      value={value}
      min={min}
      max={max}
      onChange={(e) => {
        const n = parseInt(e.target.value, 10);
        if (!Number.isNaN(n) && n >= min && n <= max) onChange(n);
      }}
      className="w-16 rounded border border-white/10 bg-black/30 px-2 py-1 text-sm tabular-nums"
    />
  );
}

function TimeInput({
  hour,
  minute,
  onChange,
}: {
  hour: number;
  minute: number;
  onChange: (hour: number, minute: number) => void;
}) {
  return (
    <input
      type="time"
      value={`${pad2(hour)}:${pad2(minute)}`}
      onChange={(e) => {
        const [h, m] = e.target.value.split(":").map((s) => parseInt(s, 10));
        if (!Number.isNaN(h) && !Number.isNaN(m)) onChange(h, m);
      }}
      className="rounded border border-white/10 bg-black/30 px-2 py-1 text-sm tabular-nums"
    />
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
  return new Date(epochMs).toLocaleString();
}
