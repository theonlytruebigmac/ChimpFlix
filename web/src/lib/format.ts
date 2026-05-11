// Display formatters for runtimes, ratings, dates.

export function formatRuntime(ms?: number | null): string | null {
  if (!ms || ms <= 0) return null;
  const totalMinutes = Math.round(ms / 60000);
  if (totalMinutes < 60) return `${totalMinutes}m`;
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return minutes === 0 ? `${hours}h` : `${hours}h ${minutes}m`;
}

export function formatRating(r?: number | null): string | null {
  if (typeof r !== "number") return null;
  return r.toFixed(1);
}

export function formatDate(ms?: number | null): string | null {
  if (!ms) return null;
  return new Date(ms).toLocaleDateString();
}
