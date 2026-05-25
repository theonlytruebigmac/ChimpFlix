/// Dev-only console logging.
///
/// Production builds shouldn't be writing noisy `console.warn` /
/// `console.error` for non-fatal events — those are operator
/// breadcrumbs that belong in the server log or a real telemetry
/// sink, not in every end-user's devtools panel.
///
/// Usage: replace `console.warn("[x] thing")` with `devWarn("[x] thing")`.
/// Same signature; no-op in prod (NODE_ENV === "production").

const isDev = process.env.NODE_ENV !== "production";

export function devWarn(...args: unknown[]): void {
  if (isDev) console.warn(...args);
}

export function devError(...args: unknown[]): void {
  if (isDev) console.error(...args);
}

export function devLog(...args: unknown[]): void {
  if (isDev) console.log(...args);
}
