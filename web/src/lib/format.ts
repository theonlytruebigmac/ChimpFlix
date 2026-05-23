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
