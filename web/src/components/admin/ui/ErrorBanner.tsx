"use client";

/// Shared error banner for admin pages. Use this wherever a save/test/
/// load action can fail and the operator needs to see why. Assertive
/// `aria-live` so screen-reader users hear the failure even if focus
/// has moved (e.g. dirty Save button → failed network call → error
/// renders out of focus). Renders nothing when `error` is null so
/// callers can drop it at the top of their layout unconditionally.
///
/// Prefer this over hand-rolled `<div className="bg-red-500/10 …">`
/// blocks — those drift in spacing / aria coverage, and screen-reader
/// users miss the failures entirely.
export function ErrorBanner({
  error,
  className,
}: {
  error: string | null | undefined;
  className?: string;
}) {
  if (!error) return null;
  return (
    <div
      role="alert"
      aria-live="assertive"
      className={`rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300 ${className ?? ""}`}
    >
      {error}
    </div>
  );
}
