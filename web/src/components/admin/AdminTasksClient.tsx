"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type ScheduledTask,
  type TaskKindInfo,
  type TaskRun,
  type TasksListResponse,
} from "@/lib/chimpflix-api";

const CRON_PRESETS: ReadonlyArray<{ label: string; value: string }> = [
  { label: "Every 2 minutes", value: "0 */2 * * * *" },
  { label: "Every 30 minutes", value: "0 */30 * * * *" },
  { label: "Hourly", value: "0 0 * * * *" },
  { label: "Every 4 hours", value: "0 0 */4 * * *" },
  { label: "Daily at 03:00", value: "0 0 3 * * *" },
  { label: "Weekly (Sun 04:00)", value: "0 0 4 * * 0" },
];

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
            <Field label="Cron expression" hint="5/6/7-field cron.">
              <input
                type="text"
                value={cron}
                onChange={(e) => setCron(e.target.value)}
                className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
              />
              <div className="mt-1 flex flex-wrap gap-1">
                {CRON_PRESETS.map((p) => (
                  <button
                    key={p.value}
                    type="button"
                    onClick={() => setCron(p.value)}
                    className="rounded border border-white/10 px-1.5 py-0.5 text-[10px] text-white/60 hover:bg-white/5"
                  >
                    {p.label}
                  </button>
                ))}
              </div>
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
        <Field label="Cron">
          <input
            type="text"
            value={cron}
            onChange={(e) => setCron(e.target.value)}
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
          />
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
