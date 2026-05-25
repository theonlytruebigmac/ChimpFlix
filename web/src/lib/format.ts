/// Cross-component formatting helpers. Tiny by design — surfaces like
/// "5 titles" / "1 title" recur across collection / genre / person /
/// library / search / admin pages, and a half-dozen variants of
/// inline `count === 1 ? "X" : "Xs"` ternaries drift over time. One
/// helper keeps them aligned.

/// Return just the noun (no count): `plural(5, "title")` → "titles".
/// Pass an explicit plural for irregular words: `plural(2, "library", "libraries")`.
export function plural(count: number, singular: string, pluralForm?: string): string {
  return count === 1 ? singular : (pluralForm ?? `${singular}s`);
}

/// Return "N noun" with N locale-formatted:
///   pluralize(1, "title") → "1 title"
///   pluralize(2400, "title") → "2,400 titles"
///   pluralize(2, "library", "libraries") → "2 libraries"
export function pluralize(
  count: number,
  singular: string,
  pluralForm?: string,
): string {
  return `${count.toLocaleString()} ${plural(count, singular, pluralForm)}`;
}

type DateInput = Date | string | number;

/// Render a Date/timestamp as "May 24, 2026, 3:45 PM". The unifying
/// helper for any UI surface that wants both date and time on one
/// line — settings rows, activity feeds, admin tables. Picks
/// `dateStyle: medium` + `timeStyle: short` for a balance between
/// readable and compact. Locale is the user agent default so a
/// non-en-US viewer sees their local convention.
export function formatDateTime(input: DateInput): string {
  return new Date(input).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

/// Render a Date/timestamp as "May 24, 2026". Use for date-only
/// contexts: "Joined May 24, 2026" / lifespan bookends / expiry
/// dates where the hour is irrelevant.
export function formatDate(input: DateInput): string {
  return new Date(input).toLocaleDateString(undefined, {
    year: "numeric",
    month: "long",
    day: "numeric",
  });
}

/// Render a Date/timestamp as "3:45 PM". Use for time-of-day-only
/// contexts where the date is already on screen (e.g. an activity
/// feed grouped by day, where each row shows just the time).
export function formatTime(input: DateInput): string {
  return new Date(input).toLocaleTimeString(undefined, {
    timeStyle: "short",
  });
}
