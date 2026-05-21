"use client";

/// Scheduled-tasks admin surface (Screen 1 from
/// `docs/pipelines/tasks-ui.html`). Registry-driven view — one row
/// per *kind* known to the binary, joining live counters with the
/// `scheduled_tasks` row that drives the sweep cadence. This is
/// the sole tasks-list surface — the legacy per-row Advanced
/// editor was folded into the detail page in the 2026-05-20
/// consolidation.

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";

import {
  admin as adminApi,
  friendlyErrorMessage,
  type OverviewKindCard,
  type OverviewKindGroup,
  type ServerSettings,
  type TaskMode,
  type TasksOverviewResponse,
  type TasksSummaryResponse,
} from "@/lib/chimpflix-api";
import {
  formatRelativeAgo,
  formatRelativeFuture,
} from "@/lib/relative-time";
import { HeroCard, Pill, type PillTone } from "./ui";

interface Props {
  initialOverview: TasksOverviewResponse;
  initialSummary: TasksSummaryResponse;
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
  initialSummary,
  initialSettings,
  initialNowMs,
}: Props) {
  const [overview, setOverview] = useState(initialOverview);
  const [summary, setSummary] = useState(initialSummary);
  const [settings, setSettings] = useState(initialSettings);
  const [nowMs, setNowMs] = useState(initialNowMs);
  const [busyKinds, setBusyKinds] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);

  /// Refresh both endpoints. Errors are surfaced inline (banner)
  /// rather than thrown so the existing render stays usable.
  const refresh = useCallback(async () => {
    try {
      const [o, s] = await Promise.all([
        adminApi.tasks.overview(),
        adminApi.tasks.summary(),
      ]);
      setOverview(o);
      setSummary(s);
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
    <div className="space-y-6">
      {error && (
        <div className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-300">
          {error}
        </div>
      )}

      <HeroStrip summary={summary} nowMs={nowMs} />

      <MaintenanceWindowCard
        settings={settings}
        onSaved={(next) => {
          setSettings(next);
          setError(null);
        }}
        onError={setError}
      />

      <div className="flex items-center justify-end gap-2 text-xs text-white/60">
        <Link
          href="/settings/admin/library/scheduled-tasks/flow"
          className="rounded border border-white/15 px-2.5 py-1 transition-colors hover:bg-white/5"
        >
          Pipeline flow
        </Link>
        <Link
          href="/settings/admin/library/scheduled-tasks/queue"
          className="rounded border border-white/15 px-2.5 py-1 transition-colors hover:bg-white/5"
        >
          Job queue
        </Link>
        <Link
          href="/settings/admin/library/scheduled-tasks/activity"
          className="rounded border border-white/15 px-2.5 py-1 transition-colors hover:bg-white/5"
        >
          Activity →
        </Link>
      </div>

      {overview.groups.map((group) => (
        <KindGroupSection
          key={group.id}
          group={group}
          busyKinds={busyKinds}
          nowMs={nowMs}
          onToggleGate={toggleGate}
        />
      ))}
    </div>
  );
}

// ─── Hero strip ────────────────────────────────────────────────────────

function HeroStrip({
  summary,
  nowMs,
}: {
  summary: TasksSummaryResponse;
  nowMs: number;
}) {
  const windowMeta = summary.next_maintenance_window_ms
    ? `opens ${formatRelativeFuture(summary.next_maintenance_window_ms, nowMs)}`
    : "currently open";
  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
      <HeroCard
        tone="info"
        label="Running"
        value={summary.running.toString()}
        meta={`${summary.queued} pending`}
      />
      <HeroCard
        tone="ok"
        label="Last 24h"
        value={summary.succeeded_24h.toLocaleString()}
        meta={passRateMeta(summary.succeeded_24h, summary.failed_24h)}
      />
      <HeroCard
        tone={summary.failed_24h > 0 ? "warn" : "muted"}
        label="Failures last 24h"
        value={summary.failed_24h.toString()}
        meta={
          summary.failed_24h === 0
            ? "all clear"
            : "review the activity log"
        }
      />
      <HeroCard
        tone="muted"
        label="Maintenance window"
        value={windowMeta}
        meta="snaps heavy tasks to the configured slot"
      />
    </div>
  );
}

function passRateMeta(succeeded: number, failed: number): string {
  const total = succeeded + failed;
  if (total === 0) return "no runs yet";
  const pct = (succeeded / total) * 100;
  return `${pct.toFixed(1)}% pass rate`;
}

// ─── Group + section + row ─────────────────────────────────────────────

function KindGroupSection({
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
  // Hide a group entirely when it has no rows after dropping empty
  // sections (legacy housekeeping group is empty on a fresh install).
  const visibleSections = group.sections.filter((s) => s.kinds.length > 0);
  if (visibleSections.length === 0) return null;
  return (
    <section>
      <div className="mb-2 flex items-baseline justify-between gap-3 px-1">
        <h2 className="text-xs font-semibold uppercase tracking-[0.08em] text-white/65">
          {group.name}
        </h2>
      </div>
      {visibleSections.map((s, i) => (
        <div key={s.id} className={i > 0 ? "mt-3" : undefined}>
          <SubgroupDivider label={s.label} count={s.kinds.length} />
          <div className="overflow-hidden rounded-lg border border-white/10 bg-white/2">
            {s.kinds.map((k) => (
              <KindRow
                key={k.name}
                card={k}
                busy={busyKinds.has(k.name)}
                nowMs={nowMs}
                onToggleGate={onToggleGate}
              />
            ))}
          </div>
        </div>
      ))}
    </section>
  );
}

function SubgroupDivider({ label, count }: { label: string; count: number }) {
  // The single "All" subgroup is just visual noise — skip it.
  if (label === "All") return null;
  return (
    <div className="mb-2 flex items-center gap-2 px-1 text-[10.5px] font-semibold uppercase tracking-[0.08em] text-white/40">
      <span>{label}</span>
      <span className="text-white/30">·</span>
      <span className="text-white/40">{count}</span>
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
  const detailHref = `/settings/admin/library/scheduled-tasks/kind/${encodeURIComponent(card.name)}`;
  // One <Link> per row instead of four. The gate toggle is a
  // sibling `<button>` outside the link, so the toggle click
  // doesn't bubble into the link navigation. Screen readers
  // announce one "open detail for X" link per row instead of
  // four — much less noise.
  //
  // Click-target geometry: the Link is the topmost interactive
  // layer (`z-10`) covering the entire row, so a click anywhere
  // on the row — name, schedule, status, chevron — navigates to
  // the detail page. The gate toggle is bumped to `z-20` to
  // stay above the Link; its click stays local. Content cells
  // are marked `pointer-events-none` so they don't block the
  // Link from receiving clicks (otherwise siblings declared
  // after the absolutely-positioned Link would sit on top in
  // DOM order and steal the click).
  return (
    <div className="group relative grid grid-cols-[44px_1fr_auto] items-center gap-3 border-b border-white/8 px-3 py-3 last:border-b-0 transition-colors hover:bg-white/3 md:grid-cols-[44px_1.4fr_200px_220px_24px]">
      <Link
        href={detailHref}
        aria-label={`Open detail for ${card.display_name}`}
        className="absolute inset-0 z-10"
      />
      <div className="relative z-20 flex items-center justify-center">
        <GateToggle card={card} busy={busy} onToggleGate={onToggleGate} />
      </div>
      <div className="pointer-events-none relative z-0 min-w-0">
        <KindNameCell card={card} />
      </div>
      <div className="pointer-events-none relative z-0">
        <ScheduleCell card={card} nowMs={nowMs} />
      </div>
      <div className="pointer-events-none relative z-0">
        <StatusCell card={card} nowMs={nowMs} />
      </div>
      <div className="pointer-events-none relative z-0">
        <ChevronCell />
      </div>
    </div>
  );
}

function GateToggle({
  card,
  busy,
  onToggleGate,
}: {
  card: OverviewKindCard;
  busy: boolean;
  onToggleGate: (kind: string, next: boolean) => Promise<void>;
}) {
  // Always-on rows render a small ✓ rather than a toggle. The
  // backend rejects PATCH attempts against Automatic kinds; we
  // avoid even surfacing the action.
  if (card.gate.locked) {
    return (
      <span
        title="Always on"
        className="inline-flex h-4.5 w-4.5 items-center justify-center rounded-full bg-emerald-500/15 text-[10px] text-emerald-300 ring-1 ring-emerald-500/30"
      >
        ●
      </span>
    );
  }
  const enabled = card.gate.enabled;
  return (
    <button
      type="button"
      role="switch"
      aria-checked={enabled}
      disabled={busy}
      onClick={() => onToggleGate(card.name, !enabled)}
      className={`relative inline-flex h-4.5 w-8 cursor-pointer items-center rounded-full border transition-colors disabled:cursor-wait ${
        enabled
          ? "border-emerald-500/70 bg-emerald-500"
          : "border-white/20 bg-white/12"
      }`}
    >
      <span
        className={`block h-3.5 w-3.5 rounded-full bg-white shadow-sm transition-transform ${
          enabled ? "translate-x-3.5" : "translate-x-0.5"
        }`}
      />
    </button>
  );
}

function KindNameCell({ card }: { card: OverviewKindCard }) {
  return (
    <div className="min-w-0">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-[13.5px] font-semibold text-white/95">
          {card.display_name}
        </span>
        <span className="rounded bg-white/8 px-1.5 py-px text-[10px] font-semibold uppercase tracking-wide text-white/55">
          {card.scope.replace("_", " ")}
        </span>
        {modeBadge(card.mode)}
      </div>
      <div className="mt-0.5 truncate font-mono text-[11.5px] text-white/45">
        {card.name}
        {card.gate.setting_key ? ` · gate ${card.gate.setting_key}` : null}
      </div>
    </div>
  );
}

function modeBadge(mode: TaskMode) {
  if (mode === "automatic") {
    return (
      <Pill tone="info" dot>
        Auto
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

function ScheduleCell({
  card,
  nowMs,
}: {
  card: OverviewKindCard;
  nowMs: number;
}) {
  const sched = card.schedule;
  if (!sched) {
    return (
      <div className="flex flex-col gap-0.5 text-[12.5px]">
        <span className="text-white/85">On-add only</span>
        <span className="text-[11.5px] text-white/45">
          no sweep scheduled
        </span>
      </div>
    );
  }
  const primary = sched.enabled
    ? `${prettyFrequency(sched.frequency)}`
    : "Sweep disabled";
  // Guard `next_at === 0` (epoch 1970 — appears for sweeps that
  // have never been ticked yet); rendering "in 56y" looks like a
  // bug.
  const hasNext = sched.enabled && sched.next_at > 0;
  const sub = hasNext
    ? `Next ${formatRelativeFuture(sched.next_at, nowMs)}`
    : sched.last_at
      ? `Last ran ${formatRelativeAgo(sched.last_at, nowMs)}`
      : sched.enabled
        ? "Awaiting first run"
        : "Never run";
  return (
    <div className="flex flex-col gap-0.5 text-[12.5px]">
      <span className="text-white/85">{primary}</span>
      <span className="text-[11.5px] text-white/45">{sub}</span>
    </div>
  );
}

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

function StatusCell({
  card,
  nowMs,
}: {
  card: OverviewKindCard;
  nowMs: number;
}) {
  // Health pill: derived from the schedule row's last_status if
  // present. In-flight count is shown as a separate info pill
  // alongside; queued count likewise. (The aggregate pill is the
  // *health verdict*, not the *current activity*.)
  const tone: PillTone =
    card.schedule?.last_status === "bad"
      ? "bad"
      : card.schedule?.last_status === "warn"
        ? "warn"
        : "ok";
  const label =
    card.schedule?.last_status === "bad"
      ? "Failing"
      : card.schedule?.last_status === "warn"
        ? "Warnings"
        : "Healthy";
  return (
    <div className="flex flex-col gap-1 text-[12.5px]">
      <div className="flex flex-wrap items-center gap-1.5">
        <Pill tone={tone} dot>
          {label}
        </Pill>
        {card.live.in_flight > 0 && (
          <Pill tone="info" dot>
            {card.live.in_flight} running
          </Pill>
        )}
        {card.live.queued > 0 && (
          <Pill tone="muted">{card.live.queued} queued</Pill>
        )}
      </div>
      <span className="text-[11.5px] text-white/45">
        {card.live.last_success_at_ms
          ? `Last ok ${formatRelativeAgo(card.live.last_success_at_ms, nowMs)}`
          : card.schedule?.last_at
            ? `Last run ${formatRelativeAgo(card.schedule.last_at, nowMs)}`
            : "No runs yet"}
      </span>
    </div>
  );
}

function ChevronCell() {
  return (
    <span aria-hidden className="hidden text-white/30 md:inline">
      ›
    </span>
  );
}

// ─── Maintenance window card ───────────────────────────────────────────

/// Operator-facing editor for `maintenance_window_start` /
/// `_end` (HH:MM). Tasks whose schedule has
/// `requires_maintenance_window = true` get their `next_run_at`
/// snapped forward into this window so heavy sweeps don't compete
/// with playback prime time. Lived on the legacy Advanced editor
/// before the consolidation — folded into the new overview so
/// there's a single place to tune scheduled tasks.
function MaintenanceWindowCard({
  settings,
  onSaved,
  onError,
}: {
  settings: ServerSettings;
  onSaved: (next: ServerSettings) => void;
  onError: (msg: string) => void;
}) {
  // Remount the editor whenever the parent's settings change so
  // the form picks up the new baseline without a useEffect-syncing
  // anti-pattern. In practice this only happens after Save (the
  // parent updates its state from the PATCH response) or if
  // someone reloads the page; either way an in-progress edit
  // surviving is not a goal here.
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
    <div className="rounded-lg border border-white/10 bg-white/2">
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-white/8 px-4 py-3">
        <div>
          <div className="text-[13.5px] font-semibold text-white/95">
            Maintenance window
          </div>
          <div className="text-xs text-white/55">
            Heavy sweeps (full scans, metadata refresh, backups) snap into
            this window so they don&rsquo;t compete with playback. Server-local
            time. End earlier than start to wrap midnight.
          </div>
        </div>
        {isActive ? (
          <Pill tone="ok" dot>
            Active now
          </Pill>
        ) : (
          <Pill tone="muted">
            Next opening {describeNextWindow(start, end)}
          </Pill>
        )}
      </div>
      <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
        <div className="flex flex-wrap items-center gap-3 text-[12.5px]">
          <label className="flex items-center gap-2">
            <span className="text-white/55">From</span>
            <input
              type="time"
              value={start}
              onChange={(e) => setStart(e.target.value)}
              disabled={saving}
              className="rounded border border-white/15 bg-black/30 px-2 py-1 tabular-nums text-white/90 focus:border-white/30 focus:outline-none"
            />
          </label>
          <label className="flex items-center gap-2">
            <span className="text-white/55">to</span>
            <input
              type="time"
              value={end}
              onChange={(e) => setEnd(e.target.value)}
              disabled={saving}
              className="rounded border border-white/15 bg-black/30 px-2 py-1 tabular-nums text-white/90 focus:border-white/30 focus:outline-none"
            />
          </label>
        </div>
        <button
          type="button"
          onClick={save}
          disabled={!dirty || saving}
          className="rounded bg-white/85 px-3 py-1.5 text-xs font-medium text-black transition-colors hover:bg-white disabled:cursor-not-allowed disabled:bg-white/20 disabled:text-white/55"
        >
          {saving ? "Saving…" : "Save window"}
        </button>
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

