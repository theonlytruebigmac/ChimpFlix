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
