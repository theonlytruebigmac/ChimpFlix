/// Pure-function relative-time formatting shared by the admin
/// scheduled-task surface (overview / activity / detail / flow).
///
/// All formatters take an explicit `nowMs` rather than reading the
/// wall clock, for two reasons:
///
///   1. React-hooks/purity. The codebase convention (see
///      MEMORY.md: feedback_react_hooks_purity_date_now) is to
///      snapshot Date.now() at fetch time and thread it through
///      props. Inline `Date.now()` calls during render trigger
///      strict-mode impurity warnings and cause SSR/CSR hydration
///      mismatches because the server's "now" differs from the
///      client's first-paint "now".
///
///   2. Consistency. With nowMs as a prop, every cell on a
///      screen uses the same reference instant, so a row that
///      shows "5m ago" doesn't sit next to one that shows
///      "5m ago" computed 80ms later.

/// Format a past timestamp as "{N}s/m/h/d ago". Returns
/// "just now" for sub-5-second deltas.
export function formatRelativeAgo(targetMs: number, nowMs: number): string {
  const delta = Math.max(nowMs - targetMs, 0);
  return formatDelta(delta, "ago");
}

/// Format a future timestamp as "in {N}s/m/h/d". Returns
/// "imminent" for sub-5-second deltas. Clamps negative deltas
/// (target already passed) to "imminent" so a slightly-stale
/// next_run_at doesn't render as "in 0s".
export function formatRelativeFuture(targetMs: number, nowMs: number): string {
  if (targetMs <= 0) return "—";
  const delta = Math.max(targetMs - nowMs, 0);
  return formatDelta(delta, "future");
}

/// Relative *calendar-day* label for an air date — "Today" / "Tomorrow" /
/// "Yesterday", a bare weekday inside the next week ("Saturday"), or a
/// "Weekday, Mon D" form further out ("Saturday, Jun 6"). Used by the local
/// calendar surfaces (the home "Coming up" rail + /calendar page).
///
/// Both arguments are epoch milliseconds. `targetMs` is an air date, which the
/// backend stores at midnight UTC and treats as a plain calendar date (TMDB /
/// TVDB give air dates with no time-of-day). We therefore read it back via its
/// UTC components — see `calendarDayDelta` — so a viewer west of UTC doesn't
/// see every air date slip a day earlier. `nowMs` is passed explicitly for the
/// same purity/consistency reasons as the formatters above (snapshot once at
/// fetch time, thread through props).
export function relativeDayLabel(targetMs: number, nowMs: number): string {
  const dayDelta = calendarDayDelta(targetMs, nowMs);
  if (dayDelta === 0) return "Today";
  if (dayDelta === 1) return "Tomorrow";
  if (dayDelta === -1) return "Yesterday";
  const d = new Date(targetMs);
  // Render the weekday/date from the air date's UTC components for the same
  // reason as the delta — the stored midnight-UTC instant *is* the calendar
  // date, so formatting it in local time would name the wrong day west of UTC.
  // Inside the next week, the weekday alone is unambiguous ("Saturday").
  if (dayDelta > 1 && dayDelta < 7) {
    return d.toLocaleDateString(undefined, { weekday: "long", timeZone: "UTC" });
  }
  // Otherwise (further out, or in the past) qualify with the date.
  return d.toLocaleDateString(undefined, {
    weekday: "long",
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });
}

/// Relative air-date label for an *upcoming* episode, in the bucketed
/// "Today / Tomorrow / In N days / Next week / In N weeks" phrasing Trakt
/// uses on its upcoming-episode surfaces. Returns `null` when the episode
/// has already aired (or airs today-or-earlier by less than a day) — callers
/// fall back to their normal display (e.g. the runtime chip) in that case.
///
/// Buckets, by calendar-day delta (so "Tomorrow" is correct across a midnight
/// boundary regardless of the wall-clock time-of-day):
///
///   * 0 days   → "Today"
///   * 1 day    → "Tomorrow"
///   * 2-6      → "In N days"
///   * 7-13     → "Next week"
///   * 14-20    → "In 2 weeks"
///   * 21-27    → "In 3 weeks"
///   * 28+      → "In N weeks"  (floor(delta / 7))
///
/// Both args are epoch milliseconds; `nowMs` is threaded explicitly for the
/// same purity/consistency reasons as the other helpers in this module. See
/// `calendarDayDelta` for why the air date is read in UTC.
export function upcomingAirLabel(
  targetMs: number,
  nowMs: number,
): string | null {
  const dayDelta = calendarDayDelta(targetMs, nowMs);
  if (dayDelta < 0) return null; // already aired
  if (dayDelta === 0) return "Today";
  if (dayDelta === 1) return "Tomorrow";
  if (dayDelta < 7) return `In ${dayDelta} days`;
  if (dayDelta < 14) return "Next week";
  if (dayDelta < 21) return "In 2 weeks";
  if (dayDelta < 28) return "In 3 weeks";
  return `In ${Math.floor(dayDelta / 7)} weeks`;
}

/// Whole-day difference between an episode's air date and "now", measured in
/// calendar days *as the viewer experiences them*.
///
/// The subtlety: `air_date` is stored at midnight UTC but represents a plain
/// calendar date (the day the episode airs, no real time-of-day), whereas
/// `nowMs` is a genuine instant. So we recover the air date from `targetMs`'s
/// UTC components (the stored date, undistorted by the viewer's offset) and
/// compare it against the viewer's LOCAL calendar date. Both triples are
/// anchored to UTC midnight purely to get a DST-free whole-day count.
///
/// Doing the target in local time instead — the old behavior — made every air
/// date slip a day earlier for viewers west of UTC: an episode airing "Jun 3"
/// (midnight UTC) rendered as the evening of Jun 2 locally, so "today" showed
/// up as "Yesterday" and the calendar's Today bucket sat empty.
///
/// Exported so calendar surfaces can classify a day group as past / today /
/// upcoming (delta < 0 / === 0 / > 0) with the same logic the labels use.
export function calendarDayDelta(targetMs: number, nowMs: number): number {
  const t = new Date(targetMs);
  const n = new Date(nowMs);
  const tMid = Date.UTC(t.getUTCFullYear(), t.getUTCMonth(), t.getUTCDate());
  const nMid = Date.UTC(n.getFullYear(), n.getMonth(), n.getDate());
  return Math.round((tMid - nMid) / 86_400_000);
}

/// Day-bucket key for grouping episodes by air *date* — a stable "YYYY-MM-DD"
/// built from the timestamp's UTC components, since `air_date` is a
/// midnight-UTC calendar date. Grouping on local components would slip the
/// bucket a day for viewers west of UTC, the same drift `calendarDayDelta`
/// avoids; this keeps the bucket and its heading on the same day.
export function airDayKey(targetMs: number): string {
  const d = new Date(targetMs);
  const y = d.getUTCFullYear();
  const m = String(d.getUTCMonth() + 1).padStart(2, "0");
  const day = String(d.getUTCDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/// Short absolute air date for the secondary line under a relative heading —
/// "Wed, Jun 4". Formatted from the air date's UTC components (timeZone: "UTC")
/// for the same plain-calendar-date reason as `relativeDayLabel`; rendering it
/// in local time would name the wrong day west of UTC.
export function airDateShort(targetMs: number): string {
  return new Date(targetMs).toLocaleDateString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });
}

/// Format a duration in milliseconds. Independent of nowMs.
export function formatDurationMs(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const sec = ms / 1000;
  if (sec < 60) return `${sec.toFixed(1)}s`;
  const min = Math.floor(sec / 60);
  const remSec = Math.floor(sec - min * 60);
  return `${min}m ${remSec}s`;
}

function formatDelta(deltaMs: number, mode: "ago" | "future"): string {
  const sec = Math.floor(deltaMs / 1000);
  if (sec < 5) return mode === "ago" ? "just now" : "imminent";
  const core = shortDelta(sec);
  return mode === "ago" ? `${core} ago` : `in ${core}`;
}

function shortDelta(sec: number): string {
  if (sec < 60) return `${sec}s`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 48) {
    const remMin = min % 60;
    return remMin > 0 ? `${hr}h ${remMin}m` : `${hr}h`;
  }
  return `${Math.floor(hr / 24)}d`;
}
