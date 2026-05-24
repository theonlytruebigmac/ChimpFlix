"use client";

import { useEffect, useRef, useState } from "react";
import { brandNameUpper } from "@/lib/env";
import {
  admin as adminApi,
  type ActivityKindHealth,
  type ActivityRunningJob,
} from "@/lib/chimpflix-api";

const POLL_INTERVAL_MS = 10_000;

/// Header chip that surfaces background-task activity at a glance —
/// gears icon at rest, red dot in the corner when any scheduled-task
/// kind has jobs in flight. Click opens a small popover listing the
/// running kinds + counts so the operator doesn't have to navigate to
/// the admin page to see what the server is chewing on.
///
/// Admin-only: hides itself when the user isn't authorized (the
/// activity endpoint 403s for non-admins and we treat that as
/// "no-op"). Polls every 10s while the page is visible — the Page
/// Visibility API pauses polling when the tab is backgrounded so we
/// don't burn cycles for a tab the operator can't see.
export function ActivityIndicator() {
  const [authorized, setAuthorized] = useState<boolean | null>(null);
  const [perKind, setPerKind] = useState<ActivityKindHealth[]>([]);
  const [runningJobs, setRunningJobs] = useState<ActivityRunningJob[]>([]);
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    let timer: number | null = null;

    async function fetchOnce() {
      try {
        const res = await adminApi.tasks.activity();
        if (cancelled) return;
        setAuthorized(true);
        setPerKind(res.per_kind);
        setRunningJobs(res.running_jobs);
      } catch {
        // 403 (non-admin) or any other failure — hide the indicator.
        if (cancelled) return;
        setAuthorized(false);
      }
    }

    function schedule() {
      if (cancelled) return;
      if (document.visibilityState !== "visible") return;
      timer = window.setTimeout(async () => {
        await fetchOnce();
        schedule();
      }, POLL_INTERVAL_MS);
    }

    function onVisibility() {
      if (timer !== null) {
        window.clearTimeout(timer);
        timer = null;
      }
      if (document.visibilityState === "visible") {
        // Re-fetch immediately when the tab regains focus so the dot
        // updates on tab-switch without waiting for the next tick.
        void fetchOnce().then(schedule);
      }
    }

    void fetchOnce().then(schedule);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      cancelled = true;
      if (timer !== null) window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, []);

  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      if (!containerRef.current?.contains(e.target as Node)) setOpen(false);
    }
    function onEsc(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onEsc);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onEsc);
    };
  }, [open]);

  if (authorized === false) return null;

  const running = perKind.filter((k) => k.in_flight > 0);
  const totalInFlight = running.reduce((n, k) => n + k.in_flight, 0);
  const isRunning = totalInFlight > 0;

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-label={
          isRunning
            ? `Activity — ${totalInFlight} task${totalInFlight === 1 ? "" : "s"} running`
            : "Activity"
        }
        aria-expanded={open}
        title={
          isRunning
            ? `${totalInFlight} background task${totalInFlight === 1 ? "" : "s"} running`
            : "Activity"
        }
        className="relative flex h-8 w-8 items-center justify-center rounded-full text-white/80 transition-colors hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-(--color-accent) focus-visible:ring-offset-2 focus-visible:ring-offset-background"
      >
        <ActivityRing spinning={isRunning} />
        <ActivityPulseIcon />
      </button>

      {open && (
        <div
          role="menu"
          className="fixed right-2 top-16 z-50 w-[calc(100vw-1rem)] max-w-sm overflow-hidden rounded-md border border-white/10 bg-(--color-surface) shadow-2xl sm:absolute sm:right-0 sm:top-full sm:mt-2 sm:w-80"
        >
          <div className="border-b border-white/10 px-4 pt-3 pb-1">
            <div className="text-[11px] font-bold uppercase tracking-[0.15em] text-(--color-accent)">
              {brandNameUpper()}
            </div>
          </div>
          <a
            href="/settings/admin/tasks"
            className="block border-b border-white/10 px-4 py-2.5 text-sm font-medium text-white/90 transition-colors hover:bg-white/5 hover:text-white focus:outline-none focus-visible:bg-white/10"
          >
            Dashboard
          </a>
          {runningJobs.length === 0 ? (
            <div className="px-4 py-6 text-center text-sm text-white/55">
              {/*
                Plex's popover stays mounted with "Dashboard" + an
                empty body when idle; mirror that — the indicator's
                own arc/dot already signals "nothing running", the
                popover just needs to confirm it on click.
              */}
              Nothing running right now.
            </div>
          ) : (
            <ul className="max-h-96 divide-y divide-white/5 overflow-y-auto">
              {runningJobs.map((job) => (
                <li key={job.id} className="px-4 py-2.5">
                  <div className="text-[12.5px] text-white/55">
                    {actionLabel(job.kind, job.display_name)}
                  </div>
                  <div className="mt-0.5 line-clamp-1 text-sm font-medium text-white">
                    {job.title ?? "—"}
                    {job.episode_code && (
                      <span className="ml-1.5 text-white/55">
                        {job.episode_code}
                      </span>
                    )}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}

/// Plex-style activity icon: an EKG/pulse line inside the chip. Stays
/// static (no internal animation) — the "is this thing working" signal
/// comes from the rotating arc rendered on top, not the icon itself.
function ActivityPulseIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M22 12h-4l-3 9L9 3l-3 9H2" />
    </svg>
  );
}

/// Orange progress arc that sweeps around the icon while a task is
/// running. Idle state: a faint full ring (matches Plex's resting
/// state). Active state: a partial accent-colored arc that rotates,
/// reading as "something is actively happening" without the visual
/// noise of a spinning gears icon.
/// Map a job kind to a Plex-style action label ("Detecting markers:")
/// for the popover entry. Falls back to the registry's `display_name`
/// (formatted as "Display name:") for kinds we haven't hand-mapped —
/// the server-side display_name is operator-readable and good enough
/// when the verb form isn't worth a special case.
function actionLabel(kind: string, displayName: string): string {
  const VERB: Record<string, string> = {
    detect_markers_file: "Detecting markers",
    analyze_loudness: "Analyzing audio",
    fetch_subtitles_item: "Fetching subtitles",
    extract_embedded_subs: "Extracting subtitles",
    refresh_metadata_item: "Refreshing metadata",
    refresh_logos_item: "Refreshing logos",
    detect_extras_item: "Finding extras",
    fetch_external_ratings: "Fetching ratings",
    bootstrap_season_refs: "Analyzing season",
  };
  const verb = VERB[kind];
  return `${verb ?? displayName}:`;
}

function ActivityRing({ spinning }: { spinning: boolean }) {
  return (
    <svg
      width="32"
      height="32"
      viewBox="0 0 32 32"
      fill="none"
      aria-hidden
      className={`absolute inset-0 ${spinning ? "animate-spin" : ""}`}
      // 2s feels like a deliberate "background task ticking" cadence —
      // fast enough to read as alive, slow enough not to fight the
      // resting feel of the rest of the header.
      style={spinning ? { animationDuration: "2s" } : undefined}
    >
      <circle
        cx="16"
        cy="16"
        r="14"
        stroke="currentColor"
        strokeOpacity={spinning ? "0.18" : "0.25"}
        strokeWidth="2"
      />
      {spinning && (
        <circle
          cx="16"
          cy="16"
          r="14"
          stroke="var(--color-accent)"
          strokeWidth="2"
          strokeLinecap="round"
          // ~30% of the circumference = a quarter-and-change sweep,
          // matching the Plex reference where the arc occupies roughly
          // the bottom-left third of the ring.
          strokeDasharray={`${2 * Math.PI * 14 * 0.3} ${2 * Math.PI * 14}`}
          transform="rotate(-90 16 16)"
        />
      )}
    </svg>
  );
}
