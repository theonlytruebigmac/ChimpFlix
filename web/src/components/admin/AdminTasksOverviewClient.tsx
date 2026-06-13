"use client";

/// Scheduled-tasks admin surface (Screen 1 from
/// `docs/pipelines/tasks-ui.html`). Registry-driven view — one row
/// per *kind* known to the binary, joining live counters with the
/// `scheduled_tasks` row that drives the sweep cadence. This is
/// the sole tasks-list surface — the legacy per-row Advanced
/// editor was folded into the detail page in the 2026-05-20
/// consolidation.
///
/// Styled with the console design system (`cf-*`) to match the
/// redesign mockup: a 5-stat job-queue hero, a registry-driven
/// scheduled-tasks table with a per-kind Enabled toggle + next-run,
/// and the maintenance-window setting card. Production's richer
/// per-kind health (status pill + in-flight/queued) is kept in the
/// table cells.

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";

import {
  admin as adminApi,
  friendlyErrorMessage,
  type JobSummary,
  type OverviewKindCard,
  type OverviewKindGroup,
  type ServerSettings,
  type TasksOverviewResponse,
} from "@/lib/chimpflix-api";
import {
  formatRelativeAgo,
  formatRelativeFuture,
} from "@/lib/relative-time";

interface Props {
  initialOverview: TasksOverviewResponse;
  /// Durable job-queue counters (queued/running/succeeded/failed/dead).
  /// Drives the 5-stat hero. Polled on the same cadence as the rest.
  initialJobsSummary: JobSummary;
  initialSettings: ServerSettings;
  /// `Date.now()` snapshot from the server fetch. Threaded into
  /// every relative-time formatter so SSR HTML and the first
  /// client paint agree (no hydration mismatch). Refreshed every
  /// polling tick.
  initialNowMs: number;
}

/// 5-second poll matches the mockup's "live" feel and the budget in
/// the backend-plan performance table (no SSE in v1).
const REFRESH_MS = 5_000;

export function AdminTasksOverviewClient({
  initialOverview,
  initialJobsSummary,
  initialSettings,
  initialNowMs,
}: Props) {
  const [overview, setOverview] = useState(initialOverview);
  const [jobsSummary, setJobsSummary] = useState(initialJobsSummary);
  const [settings, setSettings] = useState(initialSettings);
  const [nowMs, setNowMs] = useState(initialNowMs);
  const [busyKinds, setBusyKinds] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);

  /// Refresh all three endpoints. Errors are surfaced inline (banner)
  /// rather than thrown so the existing render stays usable. `jobs.summary`
  /// is the same endpoint the Queue tab polls — reused here for the hero,
  /// no new backend.
  const refresh = useCallback(async () => {
    try {
      const [o, j] = await Promise.all([
        adminApi.tasks.overview(),
        adminApi.jobs.summary(),
      ]);
      setOverview(o);
      setJobsSummary(j);
      setNowMs(Date.now());
      setError(null);
    } catch (e) {
      setError(friendlyErrorMessage(e));
    }
  }, []);

  // 5s polling for the live counters. Cleanup on unmount cancels.
  useEffect(() => {
    const id = setInterval(refresh, REFRESH_MS);
    return () => clearInterval(id);
  }, [refresh]);

  const toggleGate = useCallback(
    async (kind: string, next: boolean) => {
      // Optimistic UI: flip the local toggle so the user sees
      // immediate feedback while the PATCH is in flight. The
      // subsequent refetch will reconcile if the server rejected.
      setBusyKinds((prev) => new Set(prev).add(kind));
      setOverview((prev) => withGateFlipped(prev, kind, next));
      try {
        await adminApi.tasks.setGate(kind, next);
        await refresh();
      } catch (e) {
        // Rollback the optimistic flip.
        setOverview((prev) => withGateFlipped(prev, kind, !next));
        setError(friendlyErrorMessage(e));
      } finally {
        setBusyKinds((prev) => {
          const n = new Set(prev);
          n.delete(kind);
          return n;
        });
      }
    },
    [refresh],
  );

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

      {/* ── 5-stat job-queue hero ─────────────────────────────────── */}
      <div
        className="cf-grid"
        style={{ gridTemplateColumns: "repeat(5, 1fr)", marginBottom: 18 }}
      >
        <StatTile tone="cf-tone-blue" label="Queued" value={jobsSummary.queued} />
        <StatTile tone="cf-tone-violet" label="Running" value={jobsSummary.running} />
        <StatTile tone="cf-tone-green" label="Succeeded" value={jobsSummary.succeeded} />
        <StatTile tone="cf-tone-amber" label="Failed" value={jobsSummary.failed} />
        <StatTile tone="cf-tone-red" label="Dead" value={jobsSummary.dead} />
      </div>

      {/* ── scheduled tasks (registry-driven) ─────────────────────── */}
      {overview.groups.map((group) => (
        <KindGroupCard
          key={group.id}
          group={group}
          busyKinds={busyKinds}
          nowMs={nowMs}
          onToggleGate={toggleGate}
        />
      ))}

      {/* ── maintenance window ────────────────────────────────────── */}
      <MaintenanceWindowCard
        settings={settings}
        onSaved={(next) => {
          setSettings(next);
          setError(null);
        }}
        onError={setError}
      />
    </div>
  );
}

// ─── Hero stat tile ────────────────────────────────────────────────────

function StatTile({
  tone,
  label,
  value,
}: {
  tone: string;
  label: string;
  value: number;
}) {
  return (
    <div className={`cf-stat ${tone}`} style={{ padding: 14 }}>
      <div className="cf-stat-top">{label}</div>
      <div className="cf-stat-val" style={{ fontSize: 24 }}>
        {compactNumber(value)}
      </div>
    </div>
  );
}

/// Compact-format large counts ("12.4k") the way the mockup does;
/// small counts render verbatim.
function compactNumber(n: number): string {
  if (n < 10_000) return n.toLocaleString();
  if (n < 1_000_000) return `${(n / 1_000).toFixed(1).replace(/\.0$/, "")}k`;
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
}

// ─── Group → scheduled-tasks card/table ────────────────────────────────

function KindGroupCard({
  group,
  busyKinds,
  nowMs,
  onToggleGate,
}: {
  group: OverviewKindGroup;
  busyKinds: Set<string>;
  nowMs: number;
  onToggleGate: (kind: string, next: boolean) => Promise<void>;
}) {
  // Flatten the group's sections into a single row list — the table
  // is the unit the mockup shows. Drop empty sections (the legacy
  // housekeeping group is empty on a fresh install).
  const kinds = group.sections.flatMap((s) => s.kinds);
  if (kinds.length === 0) return null;
  return (
    <div className="cf-card">
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">{group.name}</div>
          <div className="cf-sub">
            Registry-driven — new kinds appear here automatically.
          </div>
        </div>
      </div>
      <table className="cf-table">
        <thead>
          <tr>
            <th>Task</th>
            <th>Status</th>
            <th>In flight</th>
            <th>Next run</th>
            <th>Enabled</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {kinds.map((k) => (
            <KindRow
              key={k.name}
              card={k}
              busy={busyKinds.has(k.name)}
              nowMs={nowMs}
              onToggleGate={onToggleGate}
            />
          ))}
        </tbody>
      </table>
    </div>
  );
}

function KindRow({
  card,
  busy,
  nowMs,
  onToggleGate,
}: {
  card: OverviewKindCard;
  busy: boolean;
  nowMs: number;
  onToggleGate: (kind: string, next: boolean) => Promise<void>;
}) {
  const detailHref = `/settings/admin/tasks/kind/${encodeURIComponent(card.name)}`;
  return (
    <tr>
      <td>
        <b>{card.display_name}</b>
        <div className="cf-faint" style={{ fontSize: 11 }}>
          {card.name}
          {card.gate.setting_key ? ` · gate ${card.gate.setting_key}` : null}
        </div>
      </td>
      <td>
        <StatusPill card={card} />
      </td>
      <td className="cf-num cf-mono">
        {card.live.queued} queued ·{" "}
        {card.live.in_flight > 0 ? (
          <b style={{ color: "#fff" }}>{card.live.in_flight} running</b>
        ) : (
          "0 running"
        )}
      </td>
      <td className="cf-faint">{scheduleLabel(card, nowMs)}</td>
      <td>
        <GateToggle card={card} busy={busy} onToggleGate={onToggleGate} />
      </td>
      <td className="cf-num">
        <Link className="cf-btn cf-ghost cf-tiny" href={detailHref}>
          Open
        </Link>
      </td>
    </tr>
  );
}

/// Health verdict pill (idle / running / failing) — derived from the
/// schedule row's last_status plus live activity.
function StatusPill({ card }: { card: OverviewKindCard }) {
  if (card.schedule?.last_status === "bad") {
    return (
      <span className="cf-pill cf-err">
        <span className="cf-dot" />
        Failing
      </span>
    );
  }
  if (card.schedule?.last_status === "warn") {
    return (
      <span className="cf-pill cf-warn">
        <span className="cf-dot" />
        Warnings
      </span>
    );
  }
  if (card.live.in_flight > 0) {
    return (
      <span className="cf-pill cf-info">
        <span className="cf-dot" />
        Running
      </span>
    );
  }
  return (
    <span className="cf-pill">
      <span className="cf-dot" style={{ background: "var(--ghost)" }} />
      Idle
    </span>
  );
}

/// Per-kind toggle. Automatic ("locked") kinds render a small dot
/// instead of a switch — the backend rejects PATCH against them.
function GateToggle({
  card,
  busy,
  onToggleGate,
}: {
  card: OverviewKindCard;
  busy: boolean;
  onToggleGate: (kind: string, next: boolean) => Promise<void>;
}) {
  if (card.gate.locked) {
    return (
      <span
        title="Always on"
        className="cf-pill cf-ok"
        style={{ padding: "1px 7px" }}
      >
        <span className="cf-dot" />
        Auto
      </span>
    );
  }
  const enabled = card.gate.enabled;
  return (
    <button
      type="button"
      role="switch"
      aria-checked={enabled}
      aria-label={`Enable ${card.display_name}`}
      disabled={busy}
      onClick={() => onToggleGate(card.name, !enabled)}
      className={`cf-switch${enabled ? " cf-on" : ""}`}
      style={{ verticalAlign: "middle" }}
    />
  );
}

function scheduleLabel(card: OverviewKindCard, nowMs: number): string {
  const sched = card.schedule;
  if (!sched) return "on-add only";
  if (!sched.enabled) return "sweep disabled";
  // Guard `next_at === 0` (epoch 1970 — appears for sweeps that have
  // never been ticked yet); rendering "in 56y" looks like a bug.
  if (sched.next_at > 0) return formatRelativeFuture(sched.next_at, nowMs);
  if (sched.last_at)
    return `last ran ${formatRelativeAgo(sched.last_at, nowMs)}`;
  return "awaiting first run";
}

// ─── Maintenance window card ───────────────────────────────────────────

/// Operator-facing editor for `maintenance_window_start` /
/// `_end` (HH:MM). Tasks whose schedule has
/// `requires_maintenance_window = true` get their `next_run_at`
/// snapped forward into this window so heavy sweeps don't compete
/// with playback prime time.
function MaintenanceWindowCard({
  settings,
  onSaved,
  onError,
}: {
  settings: ServerSettings;
  onSaved: (next: ServerSettings) => void;
  onError: (msg: string) => void;
}) {
  // Remount the editor whenever the parent's settings change so the
  // form picks up the new baseline without a useEffect-syncing
  // anti-pattern.
  const key = `${settings.maintenance_window_start}|${settings.maintenance_window_end}`;
  return (
    <MaintenanceWindowEditor
      key={key}
      settings={settings}
      onSaved={onSaved}
      onError={onError}
    />
  );
}

function MaintenanceWindowEditor({
  settings,
  onSaved,
  onError,
}: {
  settings: ServerSettings;
  onSaved: (next: ServerSettings) => void;
  onError: (msg: string) => void;
}) {
  const [start, setStart] = useState(settings.maintenance_window_start);
  const [end, setEnd] = useState(settings.maintenance_window_end);
  const [saving, setSaving] = useState(false);

  const dirty =
    start !== settings.maintenance_window_start ||
    end !== settings.maintenance_window_end;
  const isActive = isWithinWindow(
    settings.maintenance_window_start,
    settings.maintenance_window_end,
    new Date(),
  );

  async function save() {
    setSaving(true);
    try {
      const next = await adminApi.settings.patch({
        maintenance_window_start: start,
        maintenance_window_end: end,
      });
      onSaved(next.settings);
    } catch (e) {
      onError(friendlyErrorMessage(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="cf-card" style={{ marginBottom: 0 }}>
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">Maintenance window</div>
          <div className="cf-sub">
            Heavy sweeps (full scans, metadata refresh, backups) snap into this
            window so they don&rsquo;t compete with playback. Server-local time.
            End earlier than start to wrap midnight.
          </div>
        </div>
        <div className="cf-head-aside">
          {isActive ? (
            <span className="cf-pill cf-ok">
              <span className="cf-dot" />
              Active now
            </span>
          ) : (
            <span className="cf-pill">
              Next opening {describeNextWindow(start, end)}
            </span>
          )}
        </div>
      </div>
      <div className="cf-card-body">
        <div className="cf-row">
          <div className="cf-row-main">
            <div className="cf-row-label">Window</div>
            <div className="cf-row-help">
              Tasks scheduled outside the window wait until it opens.
            </div>
          </div>
          <div className="cf-row-control">
            <input
              type="time"
              className="cf-input cf-w-auto"
              style={{ minWidth: 110 }}
              value={start}
              onChange={(e) => setStart(e.target.value)}
              disabled={saving}
            />
            <span className="cf-faint">to</span>
            <input
              type="time"
              className="cf-input cf-w-auto"
              style={{ minWidth: 110 }}
              value={end}
              onChange={(e) => setEnd(e.target.value)}
              disabled={saving}
            />
            <button
              type="button"
              className="cf-btn cf-primary cf-sm"
              onClick={save}
              disabled={!dirty || saving}
            >
              {saving ? "Saving…" : "Save window"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

/// Parses "HH:MM". Returns minutes since midnight, or null on
/// malformed input — caller treats null as "window not configured".
function parseHhmm(s: string): number | null {
  const m = /^(\d{1,2}):(\d{2})$/.exec(s);
  if (!m) return null;
  const h = Number(m[1]);
  const mins = Number(m[2]);
  if (h < 0 || h > 23 || mins < 0 || mins > 59) return null;
  return h * 60 + mins;
}

function isWithinWindow(start: string, end: string, now: Date): boolean {
  const s = parseHhmm(start);
  const e = parseHhmm(end);
  if (s === null || e === null) return false;
  const cur = now.getHours() * 60 + now.getMinutes();
  return s <= e ? cur >= s && cur < e : cur >= s || cur < e;
}

function describeNextWindow(start: string, end: string): string {
  const s = parseHhmm(start);
  if (s === null) return "—";
  return `at ${start} (${start} → ${end})`;
}

// ─── Optimistic local mutate helpers ───────────────────────────────────

function withGateFlipped(
  overview: TasksOverviewResponse,
  kind: string,
  next: boolean,
): TasksOverviewResponse {
  return {
    ...overview,
    groups: overview.groups.map((g) => ({
      ...g,
      sections: g.sections.map((s) => ({
        ...s,
        kinds: s.kinds.map((k) =>
          k.name === kind ? { ...k, gate: { ...k.gate, enabled: next } } : k,
        ),
      })),
    })),
  };
}
